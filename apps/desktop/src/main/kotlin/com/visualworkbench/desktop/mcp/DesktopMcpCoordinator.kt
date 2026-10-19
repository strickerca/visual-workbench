package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.nio.file.Path
import java.util.Base64
import java.util.UUID

internal data class McpSelectedTarget(val selector:String,val description:String)
internal interface McpCaptureEngine {
    /** Owner action: a temporary hotkey samples the foreground target before app focus. */
    suspend fun select():McpSelectedTarget
    /** Captures only that selector's retained exact process/window identity. */
    suspend fun compile(selector:String,checkGrant:suspend()->Unit):WorkbenchCompiledPackage
    suspend fun compileVerification(selector:String,profile:McpCaptureProfile,checkGrant:suspend()->Unit):WorkbenchCompiledPackage {
        if(profile!=McpCaptureProfile(PackageTarget.Generic(),false))throw McpRefused()
        return compile(selector,checkGrant)
    }
    /** Borrow only previously retained native capture sources. Never resolve a
     * project from a public package directory or a caller-supplied filesystem path. */
    suspend fun <T> withVerificationSources(original:PackageInfo,after:PackageInfo,body:suspend(WorkbenchProject,WorkbenchProject)->T):T = throw McpRefused()
    suspend fun <T> withVerificationSource(after:PackageInfo,body:suspend(WorkbenchProject)->T):T = throw McpRefused()
    suspend fun close()
}
internal data class McpCaptureProfile(val target:PackageTarget,val includeWindowTitle:Boolean)
internal data class McpCaptureGrant(val connection:String,val selectors:List<String>,val expiresNs:Long)
internal data class McpCaptureActivity(val id:String,val connection:String,val selector:String)
internal data class McpDesktopState(
    val port:Int,val agents:List<McpAgent> = emptyList(),val packages:List<PublishedPackage> = emptyList(),
    val inbox:List<McpInboxReceipt> = emptyList(),val targets:List<McpSelectedTarget> = emptyList(),
    val grants:List<McpCaptureGrant> = emptyList(),val activeCapture:McpCaptureActivity?=null,
    val retiring:Set<String> = emptySet(),val sendPreview:McpClaudePreview?=null,
    val verificationReceipts:Set<String> = emptySet(),
    val verificationHistory:VerificationReviewHistoryPage?=null,
    val message:String?=null,val closed:Boolean=false,
)

/** Owns the real catalog, inbox, host and capture service. The app must await
 * close before releasing native runtime pins. No active editor handle is owned
 * or borrowed here: package compilation is performed under its editor mutex. */
internal class DesktopMcpCoordinator(
    private val catalog:WorkbenchPackageCatalog,private val inbox:WorkbenchMcpInbox,
    private val connection:McpConnection,private val captureEngine:McpCaptureEngine,
    private val newId:()->String={throw McpRefused()},
    private val nowNs:()->Long=System::nanoTime,
):McpOwnerActions {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private val lifecycle=Any()
    private var closing=false
    private val active=LinkedHashMap<Job,String?>()
    private var closeAttempt:CompletableDeferred<Unit>?=null
    private var closeSucceeded=false
    private var closePrepared=false
    private var connectionClosed=false;private var captureClosed=false
    private var inboxClosed=false;private var catalogClosed=false
    private val catalogGate=Mutex();private val inboxGate=Mutex();private val captureGate=Mutex()
    private val verificationTargets=McpVerificationTargets()
    private val verificationGate=Mutex()
    private val verification=McpVerificationWorkflow(catalog,inbox,captureEngine,::checkedVerificationPackage,newId)
    private val grantGate=Mutex()
    private val controls=LinkedHashMap<String,Deferred<Unit>>()
    private var batchControl:Deferred<Unit>?=null
    private val grantAdmissions=LinkedHashMap<String,Any>()
    private val grants=LinkedHashMap<String,McpCaptureGrant>()
    private var indicator:Pair<String,CompletableDeferred<Unit>>?=null
    private val mutable=MutableStateFlow(McpDesktopState(connection.port))
    val state:StateFlow<McpDesktopState> = mutable.asStateFlow()
    init {
        scope.launch { connection.agents.collect { agents ->
            mutable.update{it.copy(agents=agents)}
            val absent=synchronized(lifecycle) { val removed=grants.keys.filter { key->agents.none{it.connection==key} };removed.forEach{grants.remove(it);grantAdmissions.remove(it)};publishGrants();removed }
            absent.forEach { cancelConnection(it) }
        } }
        scope.launch { while(isActive){delay(250);val expired=synchronized(lifecycle) {
            val at=nowNs();val keys=grants.values.filter{at>=it.expiresNs}.map{it.connection};keys.forEach{grants.remove(it);grantAdmissions.remove(it)};publishGrants();keys
        };expired.forEach { cancelConnection(it) } } }
    }
    private fun key(id:String,target:String)="$id/$target"
    private fun live(session:String) { require(connection.agents.value.any{it.connection==session}) }
    private fun publishGrants(){mutable.update{it.copy(grants=grants.values.toList())}}
    private suspend fun <T> owned(session:String?=null,body:suspend()->T):T {
        val task=synchronized(lifecycle){
            if(closing || active.size>=4)throw McpRefused()
            scope.async(start=CoroutineStart.LAZY){body()}.also { job->
                active[job]=session
                job.invokeOnCompletion{synchronized(lifecycle){active.remove(job)}}
            }
        }
        task.start()
        try{return task.await()}
        catch(error:CancellationException){task.cancel();withContext(NonCancellable){task.join()};throw error}
    }
    suspend fun refresh()=owned {
        val packages=catalogGate.withLock{catalog.list()};val returns=inboxGate.withLock{inbox.list()}
        mutable.update{it.copy(packages=packages,inbox=returns)}
    }
    /** A cancelled publish may already be durable. Refresh recovers by exact ID. */
    suspend fun publish(value:WorkbenchCompiledPackage):PublishedPackage=owned { publishInside(value) }
    private suspend fun publishInside(value:WorkbenchCompiledPackage,verificationEligible:Boolean=true,beforePublish:suspend()->Unit={},beforeExpose:suspend()->Unit={}):PublishedPackage = catalogGate.withLock {
        require(key(value.info.packageId,value.info.target) !in mutable.value.retiring)
        // Original retirement uses this same gate. Recheck after acquiring it,
        // before the durable publication, as acquiring a busy gate can suspend.
        beforePublish()
        val saved=catalog.publish(value)
        // Verification routing is optional. Its bounded session registry cannot
        // turn an already durable ordinary publication into a failed result.
        val verificationAvailable=verificationEligible && try {
            verificationTargets.rememberPackage(saved.info)
        } catch(_:IllegalArgumentException) { false } catch(_:IllegalStateException) { false }
        mutable.update{state->state.copy(packages=(state.packages.filterNot{it.info.packageId==saved.info.packageId&&it.info.target==saved.info.target}+saved),sendPreview=null,
            message=if(verificationAvailable)state.message else "Package saved. Its original capture target is unavailable for verification.")}
        beforeExpose();connection.publish(Path.of(saved.directory),saved.info.manifestSha256)
        saved
    }
    /** Durable local packages are never silently exposed on service start. */
    suspend fun expose(selected:PublishedPackage)=owned {
        catalogGate.withLock {
            require(key(selected.info.packageId,selected.info.target) !in mutable.value.retiring)
            val exact=catalog.lookup(selected.info.packageId,selected.info.target,selected.info.manifestSha256)
            connection.publish(Path.of(exact.directory),exact.info.manifestSha256)
        }
    }
    suspend fun retirePackage(selected:PublishedPackage)=owned {
        val itemKey=key(selected.info.packageId,selected.info.target)
        catalogGate.withLock{verificationTargets.retire(selected.info);verification.retire(selected.info);mutable.update{it.copy(retiring=it.retiring+itemKey,sendPreview=null)}}
        // Do not hold catalogGate while draining callbacks: an unrelated capture
        // callback may be publishing, and must settle before this receipt.
        connection.unpublish(selected.info.packageId,selected.info.target,selected.info.manifestSha256)
        val receipt=catalogGate.withLock { catalog.retire(selected.info.packageId,selected.info.target,selected.info.manifestSha256) }
        mutable.update{it.copy(packages=it.packages.filterNot{p->key(p.info.packageId,p.info.target)==itemKey},
            message=if(receipt.filesRemoved)"Package unpublished and owned files removed." else "Package unpublished. Exact files remain pending supported cleanup.")}
    }
    suspend fun previewClaude(selected:PublishedPackage):McpClaudePreview=owned {
        require(selected.info.target=="claude" && key(selected.info.packageId,selected.info.target) !in mutable.value.retiring)
        connection.previewClaude(selected.info.packageId,selected.info.manifestSha256).also { preview->
            mutable.update{it.copy(sendPreview=preview,message=null)}
        }
    }
    /** Call only from the owner pressing Send on this exact displayed preview. */
    suspend fun sendClaude(session:String,displayed:McpClaudePreview)=owned {
        live(session);require(mutable.value.sendPreview==displayed)
        require(key(displayed.packageId,"claude") !in mutable.value.retiring)
        connection.pushClaude(session,displayed)
        mutable.update{it.copy(sendPreview=null,message="Written to the selected agent transport. Delivery and agent use are not confirmed.")}
    }
    suspend fun selectTarget():McpSelectedTarget=owned {
        require(mutable.value.targets.size<16)
        captureGate.withLock { captureEngine.select() }.also { target->
            require(target.selector.matches(Regex("[a-zA-Z0-9_-]{1,64}")))
            mutable.update{it.copy(targets=it.targets+target)}
        }
    }
    /** Exact connection and selected windows, at most ten minutes. Never restored. */
    suspend fun grant(session:String,selectors:List<String>,lifetimeMs:Int)=owned(session) {
        live(session);require(lifetimeMs in 1..600_000 && selectors.size in 1..16 && selectors.distinct().size==selectors.size)
        require(selectors.all{key->mutable.value.targets.any{it.selector==key}})
        val immutable=selectors.toList()
        val admission=Any()
        synchronized(lifecycle){if(closing || batchControl!=null || controls.containsKey(session))throw McpRefused();grantAdmissions[session]=admission}
        try{grantGate.withLock {
            val grant=McpCaptureGrant(session,immutable,Math.addExact(nowNs(),lifetimeMs.toLong()*1_000_000L))
            // The network call never holds the local permission lock. Revoke can
            // remove permission immediately and invalidate this late completion.
            connection.grant(session,immutable,lifetimeMs)
            live(session)
            synchronized(lifecycle){if(closing || grantAdmissions[session]!==admission)throw McpRefused();grants[session]=grant;publishGrants()}
        }}finally{synchronized(lifecycle){if(grantAdmissions[session]===admission)grantAdmissions.remove(session)}}
    }
    /** Owner control has separate admission. Local authority is removed before
     * any suspension, including when all four callback slots are occupied. */
    suspend fun revoke(session:String) {
        val task=synchronized(lifecycle){
            if(closing)throw McpRefused()
            require(session.matches(Regex("[a-f0-9]{32}")))
            grants.remove(session);grantAdmissions.remove(session);publishGrants()
            active.filterValues{it==session}.keys.forEach{it.cancel()}
            batchControl?:controls[session]?:run{
                // At most four independent controls are retained. Saturation
                // cannot restore local permission; only the remote receipt is
                // refused, and callers can retry or close the whole service.
                if(controls.size>=4)throw McpRefused()
                scope.async(start=CoroutineStart.LAZY){withContext(NonCancellable){
                    cancelConnection(session)
                    grantGate.withLock{connection.revoke(session)}
                }}.also{job->controls[session]=job;job.invokeOnCompletion{synchronized(lifecycle){if(controls[session]===job)controls.remove(session)}}}
            }
        }
        task.start()
        withContext(NonCancellable){task.await()}
    }
    /** Remove ALL current authority/admissions atomically before waiting for
     * any one capture or remote receipt. One bounded batch owns the settlement;
     * concurrent callers join it and new explicit grants wait for its end. */
    suspend fun revokeAll() {
        val task=synchronized(lifecycle){
            if(closing)throw McpRefused()
            batchControl?:run {
                val sessions=(grants.keys+grantAdmissions.keys+controls.keys+listOfNotNull(mutable.value.activeCapture?.connection)).toSet()
                // Authenticated agent inventory is bounded to 64; four pending
                // grants and four already-retiring controls allow at most 72.
                require(sessions.size<=72)
                val existing=controls.toMap()
                grants.clear();grantAdmissions.clear();publishGrants()
                val jobs=active.filterValues{it in sessions}.keys.toList()
                jobs.forEach{it.cancel()}
                scope.async<Unit>(start=CoroutineStart.LAZY){withContext(NonCancellable){
                    jobs.joinAll()
                    var failure:Throwable?=null
                    for(session in sessions){
                        try {
                            val prior=existing[session]
                            if(prior!=null){prior.start();prior.await()}
                            else grantGate.withLock{connection.revoke(session)}
                        }catch(error:Throwable){if(failure==null)failure=error}
                    }
                    failure?.let{throw it}
                }}.also{job->batchControl=job;job.invokeOnCompletion{synchronized(lifecycle){if(batchControl===job)batchControl=null}}}
            }
        }
        task.start()
        withContext(NonCancellable){task.await()}
    }

    private suspend fun cancelConnection(session:String) {
        val own=currentCoroutineContext()[Job]
        val jobs=synchronized(lifecycle){active.filter{it.value==session && it.key!==own}.keys.toList()}
        jobs.forEach{it.cancel()};withContext(NonCancellable){jobs.joinAll()}
    }
    private suspend fun checkGrant(session:String,selector:String) {
        currentCoroutineContext().ensureActive();live(session)
        synchronized(lifecycle) {
            val grant=grants[session]?:throw McpRefused()
            require(nowNs()<grant.expiresNs && selector in grant.selectors)
        }
    }
    fun indicatorDrawn(id:String) { synchronized(lifecycle){indicator?.takeIf{it.first==id}?.second?.complete(Unit)} }
    override suspend fun capture(session:String,selector:String):McpCaptureReceipt=owned(session) {
        captureInside(session,selector)
    }
    /** Session-private route association only. Resolve the exact current
     * original under the same gate used by retirement before ticket admission,
     * and again with completion. This grants no capture or alignment authority. */
    suspend fun recaptureOriginal(session:String,original:PackageInfo):McpVerificationRecapture=owned(session) {
        val (ticket,profile)=catalogGate.withLock {
            val exact=checkedVerificationPackage(original).info
            val profile=verification.profile(exact)
            verificationTargets.begin(exact) to profile
        }
        try {
            checkGrant(session,ticket.selector);verificationTargets.checkCurrent(ticket)
            val receipt=captureInside(session,ticket.selector,profile){verificationTargets.checkCurrent(ticket)}
            catalogGate.withLock {
                require(key(receipt.packageId,receipt.target) !in mutable.value.retiring)
                val matches=catalog.list().filter{it.info.packageId==receipt.packageId && it.info.target==receipt.target}
                require(matches.size==1)
                val found=matches.single()
                val after=catalog.lookup(found.info.packageId,found.info.target,found.info.manifestSha256)
                // Native lookup verifies the actual catalog/manifest binding;
                // immutable caller metadata or a routing ticket cannot replace it.
                checkedVerificationPackage(original)
                checkGrant(session,ticket.selector)
                verificationTargets.complete(ticket,after.info)
            }
        } finally {verificationTargets.abandon(ticket)}
    }
    /** Caller holds catalogGate, so retiring admission cannot race these checks. */
    private suspend fun checkedVerificationPackage(expected:PackageInfo):PublishedPackage {
        require(key(expected.packageId,expected.target) !in mutable.value.retiring)
        val exact=catalog.lookup(expected.packageId,expected.target,expected.manifestSha256)
        require(McpVerificationPackage.of(exact.info)==McpVerificationPackage.of(expected))
        return exact
    }
    private suspend fun captureInside(session:String,selector:String,profile:McpCaptureProfile?=null,checkOriginal:()->Unit={}):McpCaptureReceipt {
        suspend fun checkCapture(){checkGrant(session,selector);checkOriginal()}
        return captureGate.withLock {
            checkCapture()
            val id=UUID.randomUUID().toString();val drawn=CompletableDeferred<Unit>()
            synchronized(lifecycle){indicator=id to drawn}
            mutable.update{it.copy(activeCapture=McpCaptureActivity(id,session,selector))}
            try {
                withTimeout(2_000){drawn.await()};checkCapture()
                val compiled=if(profile==null)captureEngine.compile(selector){checkCapture()} else captureEngine.compileVerification(selector,profile){checkCapture()}
                try {
                    checkCapture()
                    val verificationEligible=try {
                        verificationTargets.rememberCapture(compiled.info,selector);true
                    } catch(_:IllegalArgumentException) { false } catch(_:IllegalStateException) { false }
                    // An unregistered/conflicting capture must never inherit an
                    // older origin merely because its canonical bytes coincide.
                    val published=publishInside(compiled,verificationEligible,beforePublish={checkCapture()}){checkCapture()};checkCapture()
                    McpCaptureReceipt(published.info.packageId,published.info.target,true,true)
                } finally {withContext(NonCancellable){compiled.close()}}
            } finally {
                synchronized(lifecycle){if(indicator?.first==id)indicator=null}
                mutable.update{if(it.activeCapture?.id==id)it.copy(activeCapture=null)else it}
            }
        }
    }
    /** Installed by the existing editor owner under its lifecycle ownership. */
    fun attachVerificationOriginal(access:McpVerificationOriginalAccess) {
        synchronized(lifecycle){check(!closing);verification.originalAccess=access}
        mutable.update{it.copy(verificationHistory=null)}
    }
    suspend fun loadVerificationHistory(after:String?=null):VerificationReviewHistoryPage=owned {
        verificationGate.withLock{catalogGate.withLock{
            verification.history(after).also{page->mutable.update{it.copy(verificationHistory=page,message=if(page.snapshots.isEmpty())"No saved reviews on this page of the open original."else"Saved reviews loaded. No capture grant was restored.")}}
        }}
    }
    suspend fun openVerificationHistory(expected:VerificationReviewSnapshot):McpInboxReceipt=owned {
        verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{
            val receipt=verification.openHistory(expected)
            mutable.update{it.copy(inbox=it.inbox.filterNot{old->old.receiptId==receipt.receiptId}+receipt,
                verificationReceipts=it.verificationReceipts+receipt.receiptId,message="Saved review reopened against checked immutable images. Capture still requires a fresh grant.")}
            receipt
        }}}
    }
    /** Called by the existing capture delivery, before compiling/exposing its
     * marked package. Retaining a route mints no connection or capture grant. */
    suspend fun rememberEditorCapture(origin:McpEditorCaptureOrigin)=owned {
        val selected=captureGate.withLock{origin.retain(captureEngine)}
        catalogGate.withLock{verificationTargets.rememberSource(origin.binding,origin.sourceAssetId,selected.selector)}
        mutable.update{it.copy(targets=if(it.targets.any{target->target.selector==selected.selector})it.targets else it.targets+selected)}
    }
    override suspend fun recaptureVerification(session:String,packageId:String,target:String,manifestHash:String):McpVerificationReceipt {
        live(session)
        val original=owned(session){catalogGate.withLock{catalog.lookup(packageId,target,manifestHash).info.also{checkedVerificationPackage(it)}}}
        val recaptured=recaptureOriginal(session,original)
        return owned(session){verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{
            withVerificationGrant(session,original){
                val result=verification.remember(recaptured,session)
                val returns=inbox.list()
                mutable.update{it.copy(inbox=returns,verificationReceipts=it.verificationReceipts+result.receiptId,message="Original window recaptured. Manual marker review is ready.")}
                result
            }
        }}}}
    }
    override suspend fun markerComparison(session:String,packageId:String,target:String,manifestHash:String,receiptId:String,receiptHash:String,number:UInt):McpMarkerComparison=owned(session) {
        live(session)
        verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{
            val original=catalog.lookup(packageId,target,manifestHash).info
            withVerificationGrant(session,original){verification.comparison(VerificationPackageKey(packageId,target,manifestHash),receiptId,receiptHash,number)}
        }}}
    }
    /** Same private selector and live grant for both agent verification tools.
     * Read-only comparison has no new OS capture and needs no second indicator. */
    private suspend fun <T> withVerificationGrant(session:String,original:PackageInfo,body:suspend()->T):T {
        val ticket=verificationTargets.begin(checkedVerificationPackage(original).info)
        try {
            checkGrant(session,ticket.selector);verificationTargets.checkCurrent(ticket)
            val result=body()
            checkGrant(session,ticket.selector);verificationTargets.checkCurrent(ticket)
            return result
        } finally {verificationTargets.abandon(ticket)}
    }
    suspend fun verificationReview(receipt:McpInboxReceipt):VerificationReviewSnapshot=owned {
        verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{verification.review(receipt)}}}
    }
    /** Explicit local owner action. No McpOwnerActions method exposes this. */
    fun verificationOperationId():String=synchronized(lifecycle){check(!closing);newId()}
    suspend fun reviewMarker(request:VerificationOwnerChange):VerificationReviewSnapshot=owned {
        verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{verification.change(request)}}}
    }
    suspend fun comparison(receipt:McpInboxReceipt,number:UInt,assumeSrgb:Boolean,allowDepth:Boolean):McpMarkerComparison=owned {
        verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{
            val review=verification.review(receipt)
            verification.comparison(review.pair.original,receipt.receiptId,receipt.receiptBlake3,number,assumeSrgb,allowDepth)
        }}}
    }
    suspend fun compileVerificationFailures(expected:VerificationReviewSnapshot,assumeSrgb:Boolean,allowDepth:Boolean):PublishedPackage?=owned {
        verificationGate.withLock {
            val compiled=catalogGate.withLock{inboxGate.withLock{verification.compile(expected,newId(),System.currentTimeMillis(),assumeSrgb,allowDepth)}}
            if(compiled==null){mutable.update{it.copy(message="All reviewed markers pass. No follow-up package was created.")};null}
            else try{publishInside(compiled,verificationEligible=false,beforePublish={inboxGate.withLock{verification.checkReview(expected)}}).also{
                mutable.update{it.copy(message="Failure-only follow-up saved against the checked recapture source. Original marker numbers are preserved in lineage.")}
            }}finally{withContext(NonCancellable){compiled.close()}}
        }
    }
    override suspend fun submitResult(session:String,packageId:String,manifestHash:String,text:String?,pngBase64:String?,note:String):String=owned(session) {
        live(session)
        val body=decodeMcpReturn(text,pngBase64,note)
        val exact=catalogGate.withLock {
            val matches=catalog.list().filter{it.info.packageId==packageId&&it.info.manifestSha256==manifestHash}
            require(matches.size==1);matches.single().also{require(key(packageId,it.info.target) !in mutable.value.retiring)}
        }
        val submission=McpInboxSubmission(newId(),packageId,exact.info.target,manifestHash,session,System.currentTimeMillis(),body.first,body.second,note)
        val receipt=inboxGate.withLock{inbox.submit(catalog,submission)}
        // Persistence may finish after cancellation; list recovers the immutable
        // receipt. No transient failure can delete that paid/external output.
        mutable.update{it.copy(inbox=it.inbox.filterNot{old->old.receiptId==receipt.receiptId}+receipt,message="An untrusted agent return is ready in Compare.")}
        receipt.receiptId
    }
    suspend fun pixels(receipt:McpInboxReceipt,side:McpInboxSide,region:McpInboxRegion,assumeSrgb:Boolean,allowDepth:Boolean):McpInboxPixels=owned {
        inboxGate.withLock{inbox.pixels(receipt.receiptId,receipt.receiptBlake3,side,region,assumeSrgb,allowDepth)}
    }
    suspend fun original(receipt:McpInboxReceipt,side:McpInboxSide):ByteArray=owned {
        inboxGate.withLock{inbox.readPng(receipt.receiptId,receipt.receiptBlake3,side)}
    }
    suspend fun retireReturn(receipt:McpInboxReceipt)=owned {
        val retired=verificationGate.withLock{catalogGate.withLock{inboxGate.withLock{inbox.retire(receipt.receiptId,receipt.receiptBlake3).also{verification.retire(receipt)}}}}
        mutable.update{it.copy(inbox=it.inbox.filterNot{old->old.receiptId==receipt.receiptId},verificationReceipts=it.verificationReceipts-receipt.receiptId,message=if(retired.filesRemoved)"Return removed." else "Return retired; exact bytes remain pending supported cleanup.")}
    }
    suspend fun close() {
        val (attempt,launch)=synchronized(lifecycle){
            closing=true
            val pending=closeAttempt?.takeUnless{it.isCompleted}
            if(pending!=null)pending to false
            else if(closeSucceeded)requireNotNull(closeAttempt) to false
            else CompletableDeferred<Unit>().also{closeAttempt=it} to true
        }
        if(launch)closeScope.launch {
            try {
                if(!closePrepared){
                    val jobs=synchronized(lifecycle){grants.clear();grantAdmissions.clear();publishGrants();active.keys.toList()+controls.values.toList()+listOfNotNull(batchControl)}
                    jobs.forEach{it.cancel()};jobs.joinAll()
                    verificationTargets.close();verification.close();scope.cancel()
                    closePrepared=true
                }
                var failure:Throwable?=null
                suspend fun settle(done:Boolean,close:suspend()->Unit,mark:()->Unit){
                    if(!done)try{close();mark()}catch(error:Throwable){val prior=failure;if(prior==null)failure=error else if(prior!==error)prior.addSuppressed(error)}
                }
                settle(connectionClosed,{connection.close()},{connectionClosed=true})
                settle(captureClosed,{captureEngine.close()},{captureClosed=true})
                settle(inboxClosed,{inbox.close()},{inboxClosed=true})
                settle(catalogClosed,{catalog.close()},{catalogClosed=true})
                failure?.let{throw it}
                mutable.update{it.copy(agents=emptyList(),grants=emptyList(),activeCapture=null,closed=true)}
                synchronized(lifecycle){closeSucceeded=true}
                attempt.complete(Unit)
            }catch(error:Throwable){attempt.completeExceptionally(error)}
        }
        withContext(NonCancellable){attempt.await()}
    }
    companion object{private val closeScope=CoroutineScope(SupervisorJob()+Dispatchers.Default)}
}
internal fun decodeMcpReturn(text:String?,encoded:String?,note:String):Pair<String?,ByteArray?> {
    require((text==null)!=(encoded==null));require(note.length<=32768 && note.toByteArray(Charsets.UTF_8).size<=32768)
    if(text!=null){require(text.isNotEmpty()&&text.length<=32768&&text.toByteArray(Charsets.UTF_8).size<=32768);return text to null}
    val value=requireNotNull(encoded)
    require(value.length in 4..5_592_408 && value.length%4==0)
    require(value.matches(Regex("[A-Za-z0-9+/]*={0,2}")))
    val bytes=Base64.getDecoder().decode(value);require(bytes.size in 1..4*1024*1024)
    require(Base64.getEncoder().encodeToString(bytes)==value)
    return null to bytes
}
