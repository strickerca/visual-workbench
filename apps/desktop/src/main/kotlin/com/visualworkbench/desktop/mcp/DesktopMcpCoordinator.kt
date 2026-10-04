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
    suspend fun close()
}
internal data class McpCaptureGrant(val connection:String,val selectors:List<String>,val expiresNs:Long)
internal data class McpCaptureActivity(val id:String,val connection:String,val selector:String)
internal data class McpDesktopState(
    val port:Int,val agents:List<McpAgent> = emptyList(),val packages:List<PublishedPackage> = emptyList(),
    val inbox:List<McpInboxReceipt> = emptyList(),val targets:List<McpSelectedTarget> = emptyList(),
    val grants:List<McpCaptureGrant> = emptyList(),val activeCapture:McpCaptureActivity?=null,
    val retiring:Set<String> = emptySet(),val sendPreview:McpClaudePreview?=null,
    val message:String?=null,val closed:Boolean=false,
)

/** Owns the real catalog, inbox, host and capture service. The app must await
 * close before releasing native runtime pins. No active editor handle is owned
 * or borrowed here: package compilation is performed under its editor mutex. */
internal class DesktopMcpCoordinator(
    private val catalog:WorkbenchPackageCatalog,private val inbox:WorkbenchMcpInbox,
    private val connection:McpConnection,private val captureEngine:McpCaptureEngine,
    private val nowNs:()->Long=System::nanoTime,
):McpOwnerActions {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private val lifecycle=Any()
    private var closing=false
    private val active=LinkedHashMap<Job,String?>()
    private val stopped=CompletableDeferred<Unit>()
    private val catalogGate=Mutex();private val inboxGate=Mutex();private val captureGate=Mutex()
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
    private suspend fun publishInside(value:WorkbenchCompiledPackage,beforeExpose:suspend()->Unit={}):PublishedPackage = catalogGate.withLock {
        require(key(value.info.packageId,value.info.target) !in mutable.value.retiring)
        val saved=catalog.publish(value)
        mutable.update{state->state.copy(packages=(state.packages.filterNot{it.info.packageId==saved.info.packageId&&it.info.target==saved.info.target}+saved),sendPreview=null)}
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
        catalogGate.withLock{mutable.update{it.copy(retiring=it.retiring+itemKey,sendPreview=null)}}
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
        captureGate.withLock {
            checkGrant(session,selector)
            val id=UUID.randomUUID().toString();val drawn=CompletableDeferred<Unit>()
            synchronized(lifecycle){indicator=id to drawn}
            mutable.update{it.copy(activeCapture=McpCaptureActivity(id,session,selector))}
            try {
                withTimeout(2_000){drawn.await()};checkGrant(session,selector)
                val compiled=captureEngine.compile(selector){checkGrant(session,selector)}
                try { checkGrant(session,selector);val published=publishInside(compiled){checkGrant(session,selector)};checkGrant(session,selector)
                    McpCaptureReceipt(published.info.packageId,published.info.target,true,true)
                } finally {withContext(NonCancellable){compiled.close()}}
            } finally {
                synchronized(lifecycle){if(indicator?.first==id)indicator=null}
                mutable.update{if(it.activeCapture?.id==id)it.copy(activeCapture=null)else it}
            }
        }
    }
    override suspend fun submitResult(session:String,packageId:String,manifestHash:String,text:String?,pngBase64:String?,note:String):String=owned(session) {
        live(session)
        val body=decodeMcpReturn(text,pngBase64,note)
        val exact=catalogGate.withLock {
            val matches=catalog.list().filter{it.info.packageId==packageId&&it.info.manifestSha256==manifestHash}
            require(matches.size==1);matches.single().also{require(key(packageId,it.info.target) !in mutable.value.retiring)}
        }
        val submission=McpInboxSubmission(UUID.randomUUID().toString(),packageId,exact.info.target,manifestHash,session,System.currentTimeMillis(),body.first,body.second,note)
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
        val retired=inboxGate.withLock{inbox.retire(receipt.receiptId,receipt.receiptBlake3)}
        mutable.update{it.copy(inbox=it.inbox.filterNot{old->old.receiptId==receipt.receiptId},message=if(retired.filesRemoved)"Return removed." else "Return retired; exact bytes remain pending supported cleanup.")}
    }
    suspend fun close() {
        val launch=synchronized(lifecycle){if(closing)false else{closing=true;true}}
        if(launch) closeScope.launch {
            var failure:Throwable?=null
            suspend fun settle(body:suspend()->Unit){try{body()}catch(error:Throwable){if(failure==null)failure=error}}
            val jobs=synchronized(lifecycle){grants.clear();grantAdmissions.clear();publishGrants();active.keys.toList()+controls.values.toList()+listOfNotNull(batchControl)};jobs.forEach{it.cancel()};jobs.joinAll()
            settle{connection.close()};settle{captureEngine.close()};settle{inbox.close()};settle{catalog.close()}
            scope.cancel();mutable.update{it.copy(agents=emptyList(),grants=emptyList(),activeCapture=null,closed=true)}
            val error=failure;if(error==null)stopped.complete(Unit)else stopped.completeExceptionally(error)
        }
        withContext(NonCancellable){stopped.await()}
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
