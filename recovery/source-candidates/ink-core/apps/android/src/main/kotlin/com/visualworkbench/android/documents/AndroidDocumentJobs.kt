package com.visualworkbench.android.documents

import android.content.*
import android.os.*
import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.selects.select
import java.io.File

internal class AndroidDocumentJobs(context: Context) : DocumentJobs {
    private val context=context.applicationContext
    override val pcRedaction=false
    private data class Quarantine(val retired:CompletableDeferred<Unit>,val file:File)
    private val admission=Mutex();private var closed=false;private var quarantine: Quarantine?=null
    override suspend fun run(request: DocumentRequest, source: ByteArray): DocumentReply = admission.withLock {
        if(closed) throw DocumentFailure(DocumentFailureKind.Unavailable)
        val previous=quarantine
        if(previous?.retired?.isCompleted==false) throw DocumentFailure(DocumentFailureKind.Isolation)
        if(previous!=null)withContext(Dispatchers.IO){check(previous.file.parentFile.canonicalFile==context.cacheDir.canonicalFile);if(previous.file.exists())check(previous.file.delete())}
        quarantine=null
        val text=documentRequestJson(request)
        val expected=withContext(Dispatchers.Default){documentSourceHash(source)}
        val parent=currentCoroutineContext()[Job]
        val connected=CompletableDeferred<IBinder>();val retired=CompletableDeferred<Unit>();val finished=CompletableDeferred<Unit>()
        val outputFile=withContext(Dispatchers.IO){File.createTempFile("vw-doc-job-",".private",context.cacheDir)}
        val inputPipe=ParcelFileDescriptor.createPipe()
        val output=ParcelFileDescriptor.open(outputFile,ParcelFileDescriptor.MODE_WRITE_ONLY or ParcelFileDescriptor.MODE_TRUNCATE)
        val reply=Messenger(object:Handler(Looper.getMainLooper()){override fun handleMessage(message:Message){if(message.what==DocumentWire.FINISHED) finished.complete(Unit)}})
        val connection=object:ServiceConnection {
            override fun onServiceConnected(name:ComponentName,binder:IBinder){try{binder.linkToDeath({retired.complete(Unit)},0);connected.complete(binder)}catch(error:Exception){connected.completeExceptionally(error);retired.complete(Unit)}}
            override fun onServiceDisconnected(name:ComponentName){retired.complete(Unit)}
            override fun onBindingDied(name:ComponentName){retired.complete(Unit)}
            override fun onNullBinding(name:ComponentName){connected.completeExceptionally(DocumentFailure(DocumentFailureKind.Isolation));retired.complete(Unit)}
        }
        var bound=false;var remote:Messenger?=null
        withContext(NonCancellable) {
            var writer:Job?=null;var monitor:Job?=null
            try {
                bound=context.bindService(Intent(context,DocumentWorkerService::class.java),connection,Context.BIND_AUTO_CREATE)
                if(!bound) throw DocumentFailure(DocumentFailureKind.Isolation)
                val binder=withTimeout(10000){connected.await()};remote=Messenger(binder)
                val message=Message.obtain(null,DocumentWire.JOB);message.replyTo=reply;message.data=Bundle().apply{putString("request",text);putParcelable("input",inputPipe[0]);putParcelable("output",output)}
                remote!!.send(message);inputPipe[0].close();output.close()
                writer=launch(Dispatchers.IO){ParcelFileDescriptor.AutoCloseOutputStream(inputPipe[1]).use{it.write(source);it.flush()}}
                monitor=launch {while(isActive){if(parent?.isActive==false){runCatching{remote?.send(Message.obtain(null,DocumentWire.CANCEL))};return@launch};delay(25)}}
                withTimeout(125000){select<Unit>{finished.onAwait{};retired.onAwait{}}}
                val result=withContext(Dispatchers.IO){outputFile.inputStream().use(DocumentWire::read)}
                withTimeout(5000){retired.await()};writer.join()
                if(parent?.isActive==false) throw CancellationException("Document worker retired after cancellation")
                withContext(Dispatchers.Default){documentReply(validateDocumentJobResult(text,expected,result))}
            } catch(error:TimeoutCancellationException){throw DocumentFailure(DocumentFailureKind.Timeout,error)}
            catch(error:DocumentJobException){throw DocumentFailure(DocumentFailureKind.Verification,error)}
            finally {
                runCatching{remote?.send(Message.obtain(null,DocumentWire.CANCEL))}
                inputPipe.forEach{runCatching{it.close()}};runCatching{output.close()}
                writer?.cancelAndJoin();monitor?.cancelAndJoin()
                if(bound) runCatching{context.unbindService(connection)}
                if(bound && !retired.isCompleted) {withTimeoutOrNull(5000){retired.await()};if(!retired.isCompleted) quarantine=Quarantine(retired,outputFile)}
                // Exact newly-created file only; output bytes have been copied or
                // refused, and worker FDs are retired before successful adoption.
                if(retired.isCompleted || !bound) withContext(Dispatchers.IO){check(outputFile.parentFile.canonicalFile==context.cacheDir.canonicalFile);if(outputFile.exists()) check(outputFile.delete())}
            }
        }
    }
    override suspend fun close(): Unit = withContext(NonCancellable){admission.withLock{closed=true;val retained=quarantine;if(retained?.retired?.isCompleted==false)throw DocumentFailure(DocumentFailureKind.Isolation);if(retained!=null)withContext(Dispatchers.IO){check(retained.file.parentFile.canonicalFile==context.cacheDir.canonicalFile);if(retained.file.exists())check(retained.file.delete())};quarantine=null}}
}
