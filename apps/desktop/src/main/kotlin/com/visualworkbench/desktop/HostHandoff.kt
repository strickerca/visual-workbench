package com.visualworkbench.desktop

import com.visualworkbench.bindings.host.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Semaphore
import java.nio.file.Files
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal class ClipboardImage(val png: ByteArray)
internal interface DesktopDragFile : AutoCloseable { val path: Path; override fun close() }
internal interface DesktopHandoff {
    suspend fun paste(assumeSrgb: Boolean): ClipboardImage
    suspend fun copy(path: Path, receipt: CompletedExport)
    suspend fun stage(path: Path, receipt: CompletedExport): DesktopDragFile
}

/** Only explicit controller actions invoke this adapter. Native polling is kept
 * off Swing, and a cancelled caller waits for the producer to settle before
 * its private input file, result object or operation token can be disposed. */
internal class NativeDesktopHandoff(private val host: HostService) : DesktopHandoff {
    private val producers = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val slots = Semaphore(2)
    private suspend fun <T> call(release: (T) -> Unit = {}, invoke: suspend (HostOperation) -> T): T {
        if (!slots.tryAcquire()) throw HandoffFailure("The Windows handoff queue is busy. Retry when current work finishes.")
        try {
            val operation = HostOperation()
            val owned = AtomicReference<T?>(null)
            try {
                val producer = producers.async { invoke(operation).also { owned.set(it) } }
                try {
                    val result = producer.await()
                    currentCoroutineContext().ensureActive()
                    owned.set(null)
                    return result
                } catch (cancel: CancellationException) {
                    operation.cancel()
                    withContext(NonCancellable) {
                        try { producer.await() } catch (_: Exception) { }
                        owned.getAndSet(null)?.let(release)
                    }
                    throw cancel
                } catch (error: HostException) {
                    throw HandoffFailure(hostMessage(error))
                }
            } finally { operation.destroy() }
        } finally { slots.release() }
    }
    override suspend fun paste(assumeSrgb: Boolean): ClipboardImage = call { operation ->
        ClipboardImage(host.readClipboardImage(ClipboardReadOptions(ClipboardReadFormat.PREFER_PNG, assumeSrgb), operation).png)
    }
    override suspend fun copy(path: Path, receipt: CompletedExport) {
        val limits = handoffLimits()
        if (receipt.bytes > limits.clipboardPngBytes || receipt.width.toULong() * receipt.height.toULong() > limits.clipboardPixels)
            throw HandoffFailure("This image exceeds Windows clipboard limits. Use Save or prepare a PNG file drag. Nothing was resized.")
        call { operation ->
            val bytes = withContext(Dispatchers.IO) {
                Files.newInputStream(path).use { input ->
                    input.readNBytes(limits.clipboardPngBytes.toInt() + 1).also {
                        if (it.size.toULong() != receipt.bytes) throw HandoffFailure("The prepared PNG changed before copy.")
                    }
                }
            }
            host.setClipboardImage(bytes, receipt.binding(), DibOptions(receipt.request.settings.allowDibDepthReduction,
                receipt.request.settings.assumeDibSrgb), operation)
        }
    }
    override suspend fun stage(path: Path, receipt: CompletedExport): DesktopDragFile = call(release = DesktopDragFile::close) { operation ->
        val native = host.stageDragPng(path.toString(), receipt.binding(), operation)
        try { NativeDragFile(native, Path.of(native.receipt().path)) }
        catch (error: Throwable) { native.release(); native.destroy(); throw error }
    }
}
private class NativeDragFile(private val native: DragFile, override val path: Path) : DesktopDragFile {
    private val closed = AtomicBoolean(false)
    override fun close() {
        if (closed.compareAndSet(false, true)) try { native.release() } finally { native.destroy() }
    }
}
private fun CompletedExport.binding(): ExportBinding = ExportBinding(blake3, sourceAsset, revision)
private fun hostMessage(error: HostException): String = when (error) {
    is HostException.DepthConversionRequired -> "This DIB representation needs an explicit depth conversion. For copy, allow the 8-bit companion; for a higher-depth pasted bitmap, convert it upstream or import the original PNG."
    is HostException.ColorAssumptionRequired -> "This bitmap has no color profile. Explicitly choose the sRGB assumption before copying or pasting."
    is HostException.ColorConversionRequired -> "The bitmap's profile cannot be represented safely. Export to RGB/sRGB explicitly or use a PNG file."
    is HostException.SizeLimit -> "This image exceeds the bounded clipboard or drag limit. Use a file export or a smaller region; nothing was resized."
    is HostException.ClipboardImageUnavailable -> "The clipboard has no PNG or supported DIB image."
    is HostException.ClipboardUnavailable -> "The clipboard is busy. Its contents were preserved; retry in a moment."
    is HostException.ClipboardPublicationFailed -> "Windows could not publish every requested format. The clipboard may contain a partial copy; retry explicitly."
    is HostException.InvalidPng, is HostException.InvalidDib, is HostException.UnsupportedDib -> "The selected clipboard image is malformed or uses an unsupported layout. No fallback image was substituted."
    is HostException.ExportBindingMismatch -> "The PNG does not match its completed source/revision receipt. Prepare it again."
    is HostException.HandoffStorage -> "The temporary drag folder could not be used safely. Existing files were preserved."
    is HostException.Busy -> "The Windows handoff queue or temporary-file budget is full. Retry after current work finishes."
    is HostException.Cancelled -> "Windows handoff cancelled before publication."
    is HostException.Timeout -> "Windows handoff timed out before publication."
    else -> "The Windows handoff service is unavailable. No successful handoff is claimed."
}
