package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.WindowsNativeFileGuard
import com.visualworkbench.shared.PublishedPackage
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.ByteArrayOutputStream
import java.math.BigDecimal
import java.nio.file.*
import java.security.MessageDigest
import java.util.UUID
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

internal class CodexRefused(val code:String):Exception("Codex handoff refused: $code")
internal data class CodexThread(val id:String,val preview:String,val status:String)
internal data class CodexSelection(val threadId:String,val model:String,val cwd:String)
internal data class CodexImage(val width:Int,val height:Int,val sha256:String)
internal data class CodexPreview(val previewId:String,val digest:String,val threadId:String,val model:String,
    val packageId:String,val manifestSha256:String,val binarySha256:String,val schemaSha256:String,
    val text:String,val detail:String,val images:List<CodexImage>)
internal data class CodexTurnReceipt(val attemptId:String,val threadId:String,val turnId:String?,val status:String){
    val terminal:Boolean get()=status in setOf("completed","failed","interrupted")
}
internal data class CodexState(val ready:Boolean=false,val busy:Boolean=false,val threads:List<CodexThread> = emptyList(),
    val cursor:String?=null,val selected:CodexSelection?=null,val preview:CodexPreview?=null,val receipt:CodexTurnReceipt?=null,
    val status:String="Select the installed codex.exe explicitly. No turn is sent while checking its schema.",val closed:Boolean=false)
private fun Map<String,Any?>.string(key:String,cap:Int=4096):String=(this[key] as? String)?.also{require(it.toByteArray(Charsets.UTF_8).size<=cap)}?:throw CodexRefused("runtime_protocol")
private fun Map<String,Any?>.integer(key:String):Int=(this[key] as? BigDecimal)?.intValueExact()?:throw CodexRefused("runtime_protocol")
@Suppress("UNCHECKED_CAST") private fun Any?.objectMap():Map<String,Any?> = this as? Map<String,Any?>?:throw CodexRefused("runtime_protocol")

/** One bounded, exact owner attempt. Terminal notification publication and
 * suspended reply delivery share the same atomic state transition. */
internal fun codexBegin(state:CodexState,displayed:CodexPreview):CodexState = state.copy(
    preview=null,receipt=CodexTurnReceipt(displayed.previewId,displayed.threadId,null,"awaiting"),
    status="Sending once. An uncertain outcome cannot be retried automatically.")
internal fun codexCompleted(state:CodexState,threadId:String,turnId:String,status:String):CodexState {
    require(status in setOf("completed","failed","interrupted"))
    val current=state.receipt?:return state
    if(current.threadId!=threadId || current.turnId!=null&&current.turnId!=turnId)return state
    if(current.terminal){require(current.status==status);return state}
    return state.copy(receipt=current.copy(turnId=turnId,status=status),
        status="Selected App Server reported turn $turnId: $status.")
}
internal suspend fun codexAccept(state:MutableStateFlow<CodexState>,displayed:CodexPreview,
    reply:suspend()->Map<String,Any?>){
    val result=reply();val attemptId=result.string("attemptId",128);val threadId=result.string("threadId",128)
    val turnId=result.string("turnId",128);val status=result.string("status",32)
    require(result["accepted"]==true&&attemptId==displayed.previewId&&threadId==displayed.threadId)
    require(status in setOf("inProgress","completed","failed","interrupted"))
    state.update{current->
        val receipt=checkNotNull(current.receipt)
        require(receipt.attemptId==attemptId&&receipt.threadId==threadId&&(receipt.turnId==null||receipt.turnId==turnId))
        if(receipt.terminal)current else current.copy(receipt=receipt.copy(turnId=turnId,status=status),
            status="App Server accepted turn $turnId: $status. Acceptance does not prove image use or task completion.")
    }
}
internal fun codexInterrupted(state:CodexState,attemptId:String?):CodexState =
    if(state.receipt?.attemptId!=attemptId||state.receipt?.terminal!=false)state
    else state.copy(status="Interrupt requested; waiting for the selected turn's completion receipt.")

/** A private owner pipe, never exposed as an MCP tool or agent callback. */
internal class CodexConnection(private val process:Process,private val event:(Map<String,Any?>)->Unit):McpOwnedService {
    private val dispatcher=Executors.newFixedThreadPool(2){r->Thread(r,"vw-codex-pipe").apply{isDaemon=true}}.asCoroutineDispatcher()
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private val output=Channel<ByteArray>(2)
    private val lock=Any();private var pending:Pair<String,CompletableDeferred<Map<String,Any?>>>?=null
    private val gate=Mutex();private val closed=AtomicBoolean();private val stopped=CompletableDeferred<Unit>()
    private val ready=CompletableDeferred<Unit>()
    init{
        scope.launch(dispatcher){try{for(bytes in output){process.outputStream.write(bytes);process.outputStream.write(10);process.outputStream.flush()}}catch(_:Exception){shutdown()}}
        scope.launch(dispatcher){try{
            val input=process.inputStream.buffered();val line=ByteArrayOutputStream()
            while(!closed.get()){val byte=input.read();if(byte<0)break;if(byte==10){require(line.size()>0);receive(McpJson.decode(line.toByteArray()));line.reset()}else{require(line.size()<4*1024*1024);line.write(byte)}}
        }catch(_:Exception){}finally{shutdown()}}
    }
    fun send(value:Map<String,Any?>){val bytes=McpJson.encode(value);require(bytes.size<=2*1024*1024);if(closed.get()||output.trySend(bytes).isFailure)throw CodexRefused("busy")}
    suspend fun awaitReady(){withTimeout(90_000){ready.await()}}
    suspend fun call(kind:String,fields:Map<String,Any?> = emptyMap()):Map<String,Any?> = gate.withLock{
        currentCoroutineContext().ensureActive();val id=UUID.randomUUID().toString();val result=CompletableDeferred<Map<String,Any?>>()
        synchronized(lock){check(!closed.get()&&pending==null);pending=id to result}
        try{send(fields+mapOf("kind" to kind,"id" to id));withTimeout(130_000){result.await()}}
        catch(error:CancellationException){withContext(NonCancellable){close()};throw error}
        finally{synchronized(lock){if(pending?.first==id)pending=null}}
    }
    private fun receive(value:Map<String,Any?>){when(value.string("kind",64)){
        "ready"->{require(value.string("binarySha256",64).matches(Regex("[0-9a-f]{64}"))&&value.string("schemaSha256",64).matches(Regex("[0-9a-f]{64}")));ready.complete(Unit);event(value)}
        "reply"->{val request=synchronized(lock){pending}?:throw CodexRefused("runtime_protocol");if(request.first!=value["id"])throw CodexRefused("runtime_protocol");if(value["ok"]==true)request.second.complete(value["result"].objectMap())else request.second.completeExceptionally(CodexRefused(value.string("code",64)))}
        "refused","runtime/refused"->{val failure=CodexRefused(value.string("code",64));ready.completeExceptionally(failure);synchronized(lock){pending?.second?.completeExceptionally(failure)};event(value)}
        "approval/refused","turn/completed"->event(value)
        else->throw CodexRefused("runtime_protocol")
    }}
    private fun shutdown(){if(!closed.compareAndSet(false,true))return;cleanup.launch(NonCancellable){
        try{val error=CodexRefused("runtime_closed");ready.completeExceptionally(error);synchronized(lock){pending?.second?.completeExceptionally(error)};output.close()
            withContext(Dispatchers.IO){stopMcpProcess(process)};scope.cancel();scope.coroutineContext[Job]?.join();dispatcher.close();stopped.complete(Unit)
        }catch(error:Throwable){stopped.completeExceptionally(error)}
    }}
    override suspend fun close(){withContext(NonCancellable){shutdown();stopped.await()}}
    companion object{private val cleanup=CoroutineScope(SupervisorJob()+Dispatchers.Default)}
}
/** The single admission slot remains held if actual process cleanup is
 * uncertain. Its retained owner pins all resources until application exit. */
internal class CodexOwnerAdmission {
    private val lock=Any();private var held:Any?=null
    fun claim():Any=synchronized(lock){if(held!=null)throw CodexRefused("owner_busy");Any().also{held=it}}
    fun release(token:Any)=synchronized(lock){check(held===token);held=null}
    fun quarantine(token:Any,owner:Any)=synchronized(lock){check(held===token);held=owner}
}
internal class CodexSettledOwner(private val admission:CodexOwnerAdmission,private val token:Any,private val cleanup:suspend()->Unit) {
    private val closing=AtomicBoolean();private val stopped=CompletableDeferred<Unit>()
    suspend fun close(){withContext(NonCancellable){
        if(closing.compareAndSet(false,true))try{cleanup();admission.release(token);stopped.complete(Unit)}
        catch(error:Throwable){admission.quarantine(token,this@CodexSettledOwner);stopped.completeExceptionally(error)}
        stopped.await()
    }}
}
private class CodexResources(token:Any):McpOwnedService {
    var bundle:DesktopMcpBundle?=null;var process:Process?=null;var connection:CodexConnection?=null
    // The cleanup closure retains this whole resource record in quarantine.
    private val owner=CodexSettledOwner(admission,token){
        val pipe=connection;if(pipe!=null)pipe.close()else process?.let{withContext(Dispatchers.IO){stopMcpProcess(it)}}
        bundle?.close();connection=null;process=null;bundle=null
    }
    override suspend fun close()=owner.close()
    companion object{val admission=CodexOwnerAdmission()}
}
private class CodexOwned(private val resources:CodexResources):McpOwnedService{
    val connection:CodexConnection get()=checkNotNull(resources.connection)
    override suspend fun close()=resources.close()
}
/** Explicit runtime selection, no executable search or fallback. Each session's
 * private inputs remain retained after exit; no package original is deleted. */
internal class DesktopCodexHandoff(private val privateParent:Path){
    private val lock=Any();private val operations=McpUiTasks();private var lifetime:McpStartLifetime<CodexOwned>?=null
    private var stopped=false;private val mutable=MutableStateFlow(CodexState());val state:StateFlow<CodexState> = mutable.asStateFlow()
    private suspend fun <T> operation(body:suspend()->T):T=operations.run{
        synchronized(lock){if(stopped||mutable.value.busy)throw CodexRefused("busy");mutable.update{it.copy(busy=true)}}
        try{body()}catch(error:CancellationException){throw error}catch(error:Exception){mutable.update{it.copy(status=explain(error))};throw error}
        finally{mutable.update{it.copy(busy=false)}}
    }
    suspend fun open(executable:Path)=operation{
        val owner=synchronized(lock){check(lifetime==null);McpStartLifetime{create(executable)}.also{lifetime=it}}
        try{owner.start();mutable.update{it.copy(ready=true,status="Installed schema checked. Choose an existing idle thread. Image Send requires an exact verified preprocessing receipt.")}}
        catch(error:Throwable){withContext(NonCancellable){owner.close()};synchronized(lock){if(lifetime===owner)lifetime=null};throw error}
    }
    private suspend fun connection():CodexConnection=synchronized(lock){checkNotNull(lifetime)}.start().connection
    suspend fun list(cursor:String?=null)=operation{val value=connection().call("threads",mapOf("cursor" to cursor));val items=(value["data"] as? List<*>)?:throw CodexRefused("runtime_protocol");require(items.size<=16)
        val threads=items.map{it.objectMap().let{item->CodexThread(item.string("id",128),item.string("preview",4096),item.string("status",32))}}
        mutable.update{it.copy(threads=threads,cursor=value["nextCursor"] as? String,status="Choose the exact thread. Active threads are refused.")}
    }
    suspend fun select(thread:CodexThread)=operation{require(state.value.threads.any{it==thread});val result=connection().call("select",mapOf("threadId" to thread.id))
        val selection=CodexSelection(result.string("threadId",128),result.string("model",256),result.string("cwd",4096));require(selection.threadId==thread.id)
        mutable.update{it.copy(selected=selection,preview=null,status="Thread selected. Prepare an exact package preview before Send.")}
    }
    suspend fun preview(value:PublishedPackage)=operation{require(value.info.target in listOf("generic","openai"));val result=connection().call("preview",mapOf("directory" to value.directory,"manifestSha256" to value.info.manifestSha256))
        val images=(result["images"] as? List<*>)?.map{it.objectMap().let{image->CodexImage(image.integer("width"),image.integer("height"),image.string("sha256",64))}}?:throw CodexRefused("runtime_protocol")
        val preview=CodexPreview(result.string("previewId",128),result.string("digest",64),result.string("threadId",128),result.string("model",256),result.string("packageId",128),result.string("manifestSha256",64),result.string("binarySha256",64),result.string("schemaSha256",64),result.string("text",2*1024*1024),result.string("detail",32),images)
        require(preview.packageId==value.info.packageId&&preview.manifestSha256==value.info.manifestSha256&&preview.threadId==state.value.selected?.threadId)
        mutable.update{it.copy(preview=preview,status="Review the exact text, images, thread and model. Preparing sends nothing.")}
    }
    suspend fun send(displayed:CodexPreview)=operation{
        require(state.value.preview==displayed);mutable.update{codexBegin(it,displayed)}
        codexAccept(mutable,displayed){connection().call("send",mapOf("displayed" to mapOf("previewId" to displayed.previewId,"digest" to displayed.digest)))}
    }
    suspend fun interrupt()=operation{val attemptId=state.value.receipt?.attemptId;connection().call("interrupt");mutable.update{codexInterrupted(it,attemptId)}}
    private fun event(value:Map<String,Any?>){when(value["kind"]){
        "turn/completed"->{val threadId=value.string("threadId",128);val turnId=value.string("turnId",128);val status=value.string("status",32)
            mutable.update{codexCompleted(it,threadId,turnId,status)}}
        "approval/refused"->mutable.update{it.copy(status="The agent requested an approval this handoff cannot grant. Use the agent's own interface; Workbench did not approve it.")}
        "refused","runtime/refused"->mutable.update{it.copy(ready=false,status=explain(CodexRefused(value.string("code",64))))}
    }}
    private suspend fun create(executable:Path):CodexOwned{
        val resources=CodexResources(CodexResources.admission.claim());var delivered=false
        try{
            withContext(Dispatchers.IO){
                require(executable.isAbsolute);val exact=executable.toRealPath();require(exact.fileName.toString().equals("codex.exe",true));val guard=WindowsNativeFileGuard();guard.check(exact)
                val bundle=DesktopMcpBundle.prepare(privateParent).also{resources.bundle=it};require(Files.isRegularFile(bundle.runtime.root.resolve("vw-codex-host.exe")))
                val base=privateParent.resolve("codex-handoffs-v1");if(!Files.exists(base,LinkOption.NOFOLLOW_LINKS))Files.createDirectory(base);guard.check(base);require(base.toRealPath()==base)
                Files.list(base).use{require(it.limit(17).count()<16)}
                val work=Files.createDirectory(base.resolve(UUID.randomUUID().toString()))
                val digest=MessageDigest.getInstance("SHA-256");guard.pin(exact).use{Files.newInputStream(exact).use{input->val bytes=ByteArray(65536);var count=0L;while(true){val n=input.read(bytes);if(n<0)break;count+=n;require(count<=256L*1024*1024);digest.update(bytes,0,n)};require(count>0)}}
                // This inventory-pinned resource is built from central measured
                // evidence. Missing profiles remain an explicit Send refusal.
                val resource=bundle.runtime.root.resolve("mcp/codex/image-profiles.json")
                val profiles=if(Files.exists(resource)){val bytes=Files.newInputStream(resource).use{it.readNBytes(65537)};require(bytes.size<=65536);McpJson.decode(bytes).let{record->require(record.keys==setOf("version","profiles")&&record["version"]==BigDecimal.ONE);record["profiles"] as? List<*>?:throw CodexRefused("invalid_image_profile")}}else emptyList<Any>()
                require(profiles.size<=16)
                val process=ProcessBuilder(bundle.runtime.root.resolve("vw-codex-host.exe").toString()).directory(bundle.runtime.root.toFile()).redirectError(ProcessBuilder.Redirect.DISCARD).apply{
                    val allowed=setOf("SystemRoot","WINDIR","USERPROFILE","APPDATA","LOCALAPPDATA","TEMP","TMP","HOMEDRIVE","HOMEPATH","PATH");environment().keys.removeIf{key->allowed.none{it.equals(key,true)}}
                }.start().also{resources.process=it}
                val connection=CodexConnection(process,::event).also{resources.connection=it}
                connection.send(mapOf("kind" to "start","executable" to exact.toString(),"binary_sha256" to digest.digest().joinToString(""){"%02x".format(it.toInt() and 255)},"work" to work.toString(),"profiles" to profiles))
            }
            checkNotNull(resources.connection).awaitReady();return CodexOwned(resources).also{delivered=true}
        }finally{if(!delivered)resources.close()}
    }
    suspend fun close(){withContext(NonCancellable){val owner=synchronized(lock){stopped=true;mutable.update{it.copy(closed=true,ready=false,preview=null)};lifetime};operations.close();owner?.close()}}
    companion object{fun explain(error:Exception):String=when((error as? CodexRefused)?.code){
        "unverified_image_preprocessing","expired_image_preprocessing"->"Send is disabled: this exact installed runtime/schema/model has no current verified unscaled-image profile. Use MCP pull, Copy or file drag; no image was sent."
        "image_would_resize"->"Send is disabled: an image exceeds the verified unscaled size/patch budget. Compile smaller derivatives explicitly."
        "thread_not_idle"->"The selected thread is active or unavailable. Choose an idle thread explicitly."
        "runtime_reply_uncertain"->"The Send outcome is uncertain. Check the selected agent thread before another handoff; this attempt will not be retried."
        else->"The installed Codex contract, package or owned process could not be verified. Send remains disabled; private staging is retained."
    }}
}
