package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Semaphore
import java.io.ByteArrayOutputStream
import java.math.BigDecimal
import java.nio.file.Files
import java.nio.file.Path
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume

/** These callbacks are implemented by the desktop owner, not by an MCP client.
 * They must settle native work on cancellation before returning. A capture
 * result is acknowledged only after the on-screen indicator and immutable
 * package publication; submitted content is saved only to the Compare inbox. */
internal interface McpOwnerActions {
    suspend fun capture(session: String, selector: String): McpCaptureReceipt
    suspend fun submitResult(session: String, packageId: String, manifestHash: String,
        text: String?, pngBase64: String?, note: String): String
}
internal data class McpCaptureReceipt(val packageId: String,val target: String,val indicatorAcknowledged: Boolean,val lossless: Boolean)
internal data class McpClaudePreview(val packageId:String,val manifestSha256:String,val folder:String,val content:String,val metadata:Map<String,String>) {
    fun wire():Map<String,Any?> = linkedMapOf("package_id" to packageId,"manifest_sha256" to manifestSha256,"package_folder" to folder,"content" to content,"meta" to metadata)
}
internal data class McpAgent(val connection: String,val bridgePid:Long)
/** Fixed nonsecret startup categories; never retain an exception message/path. */
internal enum class McpFailureStage {
    unknown, bundle_parents, bundle_manifest, bundle_extract, bundle_markers,
    bundle_inventory, bundle_verify, service_root, catalog, inbox, capture_helper, host_ready, coordinator,
}
internal class McpRefused(val stage: McpFailureStage = McpFailureStage.unknown) : Exception("The local MCP operation was refused.")

/** Terminate the owned supervisor before touching inherited pipe streams: a
 * writer may hold their monitor while a hung child refuses to drain stdin.
 * Supervisor exit closes its native kill-on-close Job and all SDK children. */
internal fun stopMcpProcess(process: Process) {
    process.destroyForcibly()
    if(!process.waitFor(5,TimeUnit.SECONDS))throw McpRefused()
    runCatching{process.outputStream.close()}
    runCatching{process.inputStream.close()}
    runCatching{process.errorStream.close()}
}

internal class DesktopMcpHost private constructor(private val process: Process, private val actions: McpOwnerActions) {
    private val io=Executors.newFixedThreadPool(2){r->Thread(r,"vw-mcp-owner-io").apply{isDaemon=true}}.asCoroutineDispatcher()
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Default)
    private val outgoing=Channel<ByteArray>(8)
    private val pending=ConcurrentHashMap<String,CompletableDeferred<Map<String,Any?>>>()
    private val ownerWork=McpOwnerWork(scope, { id, result ->
        send(if(result == null) mapOf("kind" to "owner/reply","id" to id,"ok" to false)
            else mapOf("kind" to "owner/reply","id" to id,"ok" to true,"result" to result))
    }, ::shutdownAsync)
    private val closed=AtomicBoolean(false)
    private val stopped=CompletableDeferred<Unit>()
    private val ready=CompletableDeferred<Int>()
    private val agentsMutable=MutableStateFlow<List<McpAgent>>(emptyList())
    val agents:StateFlow<List<McpAgent>> = agentsMutable
    val port:Int get()=boundPort ?: throw McpRefused()
    private var boundPort:Int?=null
    init {
        scope.launch(io) {
            try { for(bytes in outgoing){process.outputStream.write(bytes);process.outputStream.write(10);process.outputStream.flush()} }
            catch(_:Exception){shutdownAsync()}
        }
        scope.launch(io) {
            try {
                val input=process.inputStream.buffered();val line=ByteArrayOutputStream()
                while(!closed.get()) {
                    val b=input.read();if(b<0)break
                    if(b==10){require(line.size()>0);val message=McpJson.decode(line.toByteArray());line.reset();receive(message)}
                    else {require(line.size()<McpJson.MAX);line.write(b)}
                }
            } catch(_:Exception) { /* No content, paths, tokens or SDK errors enter logs. */ }
            finally { shutdownAsync() }
        }
    }
    private fun send(value:Map<String,Any?>) {
        if(closed.get() || outgoing.trySend(McpJson.encode(value)).isFailure) throw McpRefused()
    }
    private suspend fun call(kind:String,fields:Map<String,Any?>):Map<String,Any?> {
        if(pending.size>=8)throw McpRefused()
        val id=UUID.randomUUID().toString();val completion=CompletableDeferred<Map<String,Any?>>()
        pending[id]=completion
        try { send(fields+mapOf("kind" to kind,"id" to id));return withTimeout(30_000){completion.await()} }
        finally { pending.remove(id) }
    }
    suspend fun publish(directory:Path,manifestSha256:String):Map<String,Any?> {
        require(directory.isAbsolute && manifestSha256.matches(Regex("[0-9a-f]{64}")))
        return call("publish",mapOf("directory" to directory.toString(),"manifest_sha256" to manifestSha256))
    }
    suspend fun unpublish(packageId:String,target:String,manifestSha256:String) {
        val receipt=call("unpublish",mapOf("package_id" to packageId,"target" to target,"manifest_sha256" to manifestSha256))
        require(receipt["id"]==packageId && receipt["target"]==target && receipt["manifest_sha256"]==manifestSha256 && receipt["unpublished"]==true && receipt["readers_drained"]==true && receipt["owner_callbacks_drained"]==true)
    }
    suspend fun previewClaude(packageId:String,manifestSha256:String):McpClaudePreview {
        val p=call("preview/claude",mapOf("package_id" to packageId,"manifest_sha256" to manifestSha256))
        require(p["package_id"]==packageId && p["manifest_sha256"]==manifestSha256)
        val folder=p["package_folder"] as? String ?: throw McpRefused();val content=p["content"] as? String ?: throw McpRefused()
        require(folder.toByteArray(Charsets.UTF_8).size<=4096 && content.toByteArray(Charsets.UTF_8).size<=8192)
        val raw=p["meta"] as? Map<*,*> ?: throw McpRefused();require(raw.keys==setOf("package_id","manifest_sha256","package_folder","resource","marker_count"))
        val metadata=linkedMapOf<String,String>();raw.forEach{(k,v)->metadata[k as? String ?: throw McpRefused()]=v as? String ?: throw McpRefused()}
        return McpClaudePreview(packageId,manifestSha256,folder,content,metadata.toMap())
    }
    suspend fun grantCapture(connection:String,selectors:List<String>,lifetimeMs:Int) {
        require(agents.value.any{it.connection==connection} && selectors.size in 1..16 && lifetimeMs in 1..600_000)
        require(selectors.all{it.matches(Regex("[a-zA-Z0-9_-]{1,64}"))})
        call("grant",mapOf("connection" to connection,"selectors" to selectors,"lifetime_ms" to lifetimeMs))
    }
    suspend fun revokeCapture(connection:String){call("revoke",mapOf("connection" to connection))}
    /** A separate explicit UI action; publishing never invokes this method. */
    suspend fun pushClaude(connection:String,displayed:McpClaudePreview):Map<String,Any?> = call("push/claude",mapOf("connection" to connection,"package_id" to displayed.packageId,"manifest_sha256" to displayed.manifestSha256,"displayed" to displayed.wire()))
    private fun receive(message:Map<String,Any?>) {
        fun str(name:String)=message[name] as? String ?: throw McpRefused()
        when(str("kind")) {
            "ready" -> {val p=(message["port"] as? BigDecimal)?.intValueExact() ?: throw McpRefused();require(p in 1..65535 && boundPort==null);boundPort=p;ready.complete(p)}
            "control/reply" -> {val wait=pending[str("id")] ?: return;if(message["ok"]!=true)wait.completeExceptionally(McpRefused()) else {
                @Suppress("UNCHECKED_CAST") val result=message["result"] as? Map<String,Any?> ?: throw McpRefused();wait.complete(result)}}
            "agent/open" -> {val id=str("connection");val pid=(message["bridge_pid"] as? BigDecimal)?.longValueExact() ?: throw McpRefused();require(pid in 1..0xffff_ffffL && id.matches(Regex("[a-f0-9]{32}")) && agentsMutable.value.size<4 && agentsMutable.value.none{it.connection==id});agentsMutable.value=agentsMutable.value+McpAgent(id,pid)}
            "agent/closed" -> {val id=str("connection");agentsMutable.value=agentsMutable.value.filterNot{it.connection==id}}
            "owner/cancel" -> ownerWork.cancel(str("id"))
            "owner/request" -> {
                val id=str("id")
                @Suppress("UNCHECKED_CAST") val payload=message["payload"] as? Map<String,Any?> ?: throw McpRefused()
                val method=str("method")
                val accepted=ownerWork.start(id) {
                    val session=payload["session"] as? String ?: throw McpRefused()
                    withTimeout(30_000){when(method){
                        "capture" -> {require(agents.value.any{it.connection==session});val receipt=actions.capture(session,payload["selector"] as? String ?: throw McpRefused());
                            mapOf("package_id" to receipt.packageId,"target" to receipt.target,"indicator_acknowledged" to receipt.indicatorAcknowledged,"lossless" to receipt.lossless)}
                        "submit_result" -> {val receipt=actions.submitResult(session,payload["package_id"] as? String ?: throw McpRefused(),payload["manifest_sha256"] as? String ?: throw McpRefused(),payload["text"] as? String,payload["image"] as? String,payload["note"] as? String ?: throw McpRefused());mapOf("persisted" to true,"receipt_id" to receipt)}
                        else -> throw McpRefused()
                    }}
                }
                if(!accepted)send(mapOf("kind" to "owner/reply","id" to id,"ok" to false))
            }
            else -> throw McpRefused()
        }
    }
    private fun shutdownAsync() {
        if(!closed.compareAndSet(false,true))return
        startup.launch(NonCancellable) {
            var reaped=false
            try {
                ready.completeExceptionally(McpRefused());pending.values.forEach{it.completeExceptionally(McpRefused())};pending.clear()
                val jobs=ownerWork.seal();jobs.forEach{it.cancel()};outgoing.close()
                withContext(Dispatchers.IO){stopMcpProcess(process)};reaped=true
                jobs.joinAll();scope.cancel();io.close();agentsMutable.value=emptyList();stopped.complete(Unit)
            } catch(_:Exception){stopped.completeExceptionally(McpRefused())}
            finally {if(reaped)admission.release()}
        }
    }
    suspend fun shutdown(){withContext(NonCancellable){shutdownAsync();stopped.await()}}
    companion object {
        private val startup=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        private val admission=Semaphore(1)
        suspend fun open(runtime:McpRuntime,stateDirectory:Path,actions:McpOwnerActions):DesktopMcpHost = suspendCancellableCoroutine { continuation ->
            if(!admission.tryAcquire()){continuation.resumeWith(Result.failure(McpRefused()));return@suspendCancellableCoroutine}
            val producer=startup.launch(start=CoroutineStart.ATOMIC) {
                var host:DesktopMcpHost?=null
                try {
                    require(stateDirectory.isAbsolute && Files.isDirectory(stateDirectory))
                    val process=ProcessBuilder(runtime.executable.toString()).directory(runtime.root.toFile()).redirectError(ProcessBuilder.Redirect.DISCARD).apply{environment().clear()}.start()
                    val created=try { DesktopMcpHost(process,actions) } catch(error:Exception) {process.destroyForcibly();process.waitFor(5,TimeUnit.SECONDS);throw error}
                    host=created
                    created.send(mapOf("kind" to "start","state_directory" to stateDirectory.toString()))
                    withTimeout(30_000){created.ready.await()}
                    continuation.resume(created){_,undelivered,_->undelivered.shutdownAsync()};host=null
                } catch(_:Exception){host?.shutdownAsync() ?: admission.release();if(continuation.isActive)continuation.resumeWith(Result.failure(McpRefused()))}
            }
            continuation.invokeOnCancellation{producer.cancel()}
        }
    }
}
