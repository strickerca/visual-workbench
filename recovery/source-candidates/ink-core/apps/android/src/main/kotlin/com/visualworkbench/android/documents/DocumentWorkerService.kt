package com.visualworkbench.android.documents

import android.app.Service
import android.content.Intent
import android.os.*
import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.workbenchCore
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** No app UID, storage/network permission, source path or destination URI.
 * Each bind owns at most one job and ends with this process's death. */
public class DocumentWorkerService : Service() {
    private val claimed=AtomicBoolean(false)
    private val nativeWorker=Executors.newSingleThreadExecutor()
    private val watchdog=Executors.newSingleThreadScheduledExecutor()
    private fun retire() {Process.killProcess(Process.myPid())}
    private val messenger=Messenger(object: Handler(Looper.getMainLooper()) {
        @Suppress("DEPRECATION") override fun handleMessage(message: Message) {
            if(message.sendingUid!=applicationInfo.uid) return
            if(message.what==DocumentWire.CANCEL) {retire();return}
            if(message.what!=DocumentWire.JOB || !claimed.compareAndSet(false,true)) {retire();return}
            val input=message.data.getParcelable<ParcelFileDescriptor>("input")
            val output=message.data.getParcelable<ParcelFileDescriptor>("output")
            val request=message.data.getString("request")
            val reply=message.replyTo
            if(input==null || output==null || request==null || request.length>65536 || reply==null) {input?.close();output?.close();retire();return}
            try {reply.binder.linkToDeath({retire()},0)} catch(_:RemoteException) {input.close();output.close();retire();return}
            nativeWorker.execute {
                var result: DocumentJobResult?=null;var failure: String?=null
                try {
                    // Native repeats the actual reserved-UID test before dlopen.
                    val uid=Process.myUid()%100000;check(uid in 90000..99999)
                    val pin=assets.open("vw-document-android.sha256").use{it.readNBytes(1025)}.toString(Charsets.US_ASCII).trim()
                    val match=Regex("([0-9a-f]{64}) ([1-9][0-9]{0,8}) libpdfium\\.so").matchEntire(pin) ?: throw IllegalStateException("Unadmitted runtime")
                    val library=File(applicationInfo.nativeLibraryDir,"libpdfium.so")
                    check(library.isFile && library.length()==match.groupValues[2].toLong() && library.canonicalFile==library.absoluteFile)
                    workbenchCore() // Loads only this APK's generated binding library.
                    val source=ParcelFileDescriptor.AutoCloseInputStream(input).use(DocumentWire::source)
                    result=runIsolatedAndroidDocumentJob(library.absolutePath,match.groupValues[1].chunked(2).map{it.toInt(16).toByte()}.toByteArray(),request,source)
                } catch(error: DocumentJobException) {failure=when(error){is DocumentJobException.Unavailable->"Unavailable";is DocumentJobException.Invalid->"Invalid";is DocumentJobException.Limit->"Limit";is DocumentJobException.Busy->"Busy";is DocumentJobException.Cancelled->"Cancelled";is DocumentJobException.Timeout->"Timeout";is DocumentJobException.Isolation->"Isolation";is DocumentJobException.Verification->"Verification";is DocumentJobException.Password->"Password";is DocumentJobException.Unsupported->"Unsupported";is DocumentJobException.Malformed->"Malformed";is DocumentJobException.Worker->"Worker"}}
                catch(_:Exception) {failure="Unavailable"}
                finally {
                    runCatching{input.close()}
                    try {ParcelFileDescriptor.AutoCloseOutputStream(output).use{DocumentWire.write(it,result,failure)}} catch(_:Exception) {runCatching{output.close()}}
                    runCatching{reply.send(Message.obtain(null,DocumentWire.FINISHED))}
                    retire()
                }
            }
        }
    })
    override fun onBind(intent: Intent): IBinder {
        watchdog.schedule({retire()},120,TimeUnit.SECONDS)
        return messenger.binder
    }
    override fun onUnbind(intent: Intent): Boolean {retire();return false}
    override fun onDestroy() {nativeWorker.shutdownNow();watchdog.shutdownNow();super.onDestroy()}
}
