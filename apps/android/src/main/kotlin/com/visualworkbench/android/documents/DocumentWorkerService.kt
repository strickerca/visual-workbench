package com.visualworkbench.android.documents

import android.app.Service
import android.content.Intent
import android.os.*
import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*
import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** One job in a separately registered isolated process. Registration, approved
 * native packaging, and native hard memory/UID policy are root prerequisites;
 * this service is not itself proof of those policies. */
public class DocumentWorkerService : Service() {
    private val claimed = AtomicBoolean(false)
    private val worker = Executors.newSingleThreadExecutor()
    private val watchdog = Executors.newSingleThreadScheduledExecutor()
    private fun isolated(): Boolean = Process.myUid() % 100000 in 90000..99999
    private fun retire() {
        // A manifest mistake must never kill the ordinary app process.
        if (isolated()) Process.killProcess(Process.myPid()) else stopSelf()
    }
    private val messenger = Messenger(object : Handler(Looper.getMainLooper()) {
        @Suppress("DEPRECATION") override fun handleMessage(message: Message) {
            if (!isolated() || message.sendingUid != applicationInfo.uid) return
            if (message.what == DocumentWire.CANCEL) { retire(); return }
            if (message.what != DocumentWire.JOB || !claimed.compareAndSet(false, true)) { retire(); return }
            val input = message.data.getParcelable<ParcelFileDescriptor>("input")
            val output = message.data.getParcelable<ParcelFileDescriptor>("output")
            val request = message.data.getString("request")
            val needsPdfium = message.data.getBoolean("pdfium", true)
            val reply = message.replyTo
            if (input == null || output == null || request == null || request.length > 65536 ||
                request.toByteArray(Charsets.UTF_8).size > 65536 || reply == null) {
                runCatching { input?.close() }; runCatching { output?.close() }; retire(); return
            }
            try { reply.binder.linkToDeath({ retire() }, 0) }
            catch (_: RemoteException) { input.close(); output.close(); retire(); return }
            worker.execute {
                var result: DocumentWireResult? = null
                var failure: DocumentFailureKind? = null
                try {
                    val source = ParcelFileDescriptor.AutoCloseInputStream(input).use(DocumentWire::source)
                    workbenchCore() // Loads only the packaged, generated core binding library.
                    val (library, hash) = if (needsPdfium) pdfium() else "" to ByteArray(0)
                    // Native must independently classify the validated operation:
                    // an empty library is allowed ONLY for pure SVG, never PDF.
                    // It also repeats UID/artifact/hard-limit checks before dlopen.
                    val native = runIsolatedAndroidDocumentJob(library, hash, request, source)
                    result = DocumentWireResult(native.sourceBlake3, native.metadataJson, native.bytes, native.outputBlake3)
                } catch (error: DocumentJobException) { failure = nativeDocumentFailure(error).kind }
                catch (error: DocumentFailure) { failure = error.kind }
                catch (_: Exception) { failure = DocumentFailureKind.Worker }
                finally {
                    runCatching { input.close() }
                    try { ParcelFileDescriptor.AutoCloseOutputStream(output).use { DocumentWire.write(it, result, failure) } }
                    catch (_: Exception) { runCatching { output.close() } }
                    runCatching { reply.send(Message.obtain(null, DocumentWire.FINISHED)) }
                    retire()
                }
            }
        }
    })

    private fun pdfium(): Pair<String, ByteArray> {
        val bytes = assets.open("vw-document-android.sha256").use { it.readNBytes(1025) }
        if (bytes.size !in 1..1024 || bytes.any { it.toInt() !in 32..126 && it.toInt() != 10 })
            throw DocumentFailure(DocumentFailureKind.Verification)
        val pin = bytes.toString(Charsets.US_ASCII).trim()
        val match = Regex("([0-9a-f]{64}) ([1-9][0-9]{0,8}) libpdfium\\.so").matchEntire(pin)
            ?: throw DocumentFailure(DocumentFailureKind.Verification)
        val size = match.groupValues[2].toLong()
        if (size > 128L * 1024 * 1024) throw DocumentFailure(DocumentFailureKind.Limit)
        val library = File(applicationInfo.nativeLibraryDir, "libpdfium.so")
        if (!library.isFile || library.length() != size || library.canonicalFile != library.absoluteFile)
            throw DocumentFailure(DocumentFailureKind.Unavailable)
        return library.absolutePath to match.groupValues[1].chunked(2).map { it.toInt(16).toByte() }.toByteArray()
    }

    override fun onBind(intent: Intent): IBinder? {
        if (!isolated()) { stopSelf(); return null }
        watchdog.schedule({ retire() }, 120, TimeUnit.SECONDS)
        return messenger.binder
    }
    override fun onUnbind(intent: Intent): Boolean { retire(); return false }
    override fun onDestroy() { worker.shutdownNow(); watchdog.shutdownNow(); retire(); super.onDestroy() }
}
