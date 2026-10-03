@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package com.visualworkbench.android.editor

import android.content.ContentResolver
import android.net.Uri
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import android.system.StructPollfd
import com.visualworkbench.shared.setAndroidDescriptorNonBlocking
import kotlinx.coroutines.*
import java.io.FileDescriptor
import java.io.IOException
import java.io.InterruptedIOException
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.Future
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.coroutines.resumeWithException

/** Binder calls and provider FD syscalls are isolated in a bounded pool. A
 * provider that ignores cancellation cannot retain the caller or create
 * unbounded workers. Each active syscall owns its duplicate FD and private
 * buffer until it returns; late results never access caller buffers. */
internal class ProviderIo(private val resolver: ContentResolver) {
    suspend fun <T> perform(operation: () -> T): T = call { operation() }
    suspend fun open(uri: Uri, mode: String): ParcelFileDescriptor = call({ it.close() }) { signal ->
        resolver.openFileDescriptor(uri, mode, signal) ?: throw IOException("Image provider returned no file")
    }

    suspend fun title(uri: Uri): String = call { signal ->
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null, signal)?.use { cursor ->
            if (cursor.moveToFirst()) cursor.getString(0)?.take(120) else null
        }?.takeIf { it.isNotBlank() } ?: "Imported image"
    }

    private suspend fun <T> call(close: (T) -> Unit = {}, operation: (CancellationSignal) -> T): T = try {
        withTimeout(15_000) { suspendCancellableCoroutine { continuation ->
        val signal = CancellationSignal()
        val started = AtomicBoolean(false)
        val future = AtomicReference<Future<*>?>()
        continuation.invokeOnCancellation {
            future.get()?.cancel(true)
            workers.purge()
            if (started.get()) runCatching { cancellations.execute { runCatching { signal.cancel() } } }
        }
        try {
            val submitted = workers.submit {
                started.set(true)
                if (!continuation.isActive) return@submit
                try {
                    val value = operation(signal)
                    continuation.resume(value, onCancellation = { _, lost, _ -> runCatching { close(lost) } })
                } catch (error: Exception) { continuation.resumeWithException(error) }
            }
            future.set(submitted)
            if (!continuation.isActive) { submitted.cancel(true); workers.purge() }
        } catch (_: java.util.concurrent.RejectedExecutionException) {
            continuation.resumeWithException(RasterTransferFailure("The image provider is not responding. Close its pending request or try another provider."))
        }
        } }
    } catch (_: TimeoutCancellationException) {
        throw RasterTransferFailure("The image provider did not respond in time. The original is preserved; try another provider.")
    }

    fun nonBlocking(descriptor: ParcelFileDescriptor) = setAndroidDescriptorNonBlocking(descriptor)

    suspend fun read(descriptor: FileDescriptor, bytes: ByteArray): Int {
        require(bytes.isNotEmpty() && bytes.size <= CHUNK_LIMIT)
        val privateBytes = ByteArray(bytes.size)
        val count = descriptorCall(descriptor) { owned, signal ->
            while (true) {
                waitReady(owned, OsConstants.POLLIN, signal)
                try { return@descriptorCall Os.read(owned, privateBytes, 0, privateBytes.size) }
                catch (error: ErrnoException) { if (error.errno != OsConstants.EAGAIN && error.errno != OsConstants.EINTR) throw error }
            }
            @Suppress("UNREACHABLE_CODE") 0
        }
        currentCoroutineContext().ensureActive()
        if (count > 0) privateBytes.copyInto(bytes, endIndex = count)
        return count
    }

    suspend fun write(descriptor: FileDescriptor, bytes: ByteArray, length: Int) {
        require(length in 0..minOf(bytes.size, CHUNK_LIMIT))
        val privateBytes = bytes.copyOf(length)
        descriptorCall(descriptor) { owned, signal ->
            var offset = 0
            while (offset < privateBytes.size) {
                waitReady(owned, OsConstants.POLLOUT, signal)
                try {
                    val count = Os.write(owned, privateBytes, offset, privateBytes.size - offset)
                    if (count <= 0) throw IOException("Image provider stopped accepting output")
                    offset += count
                } catch (error: ErrnoException) { if (error.errno != OsConstants.EAGAIN && error.errno != OsConstants.EINTR) throw error }
            }
        }
    }

    private suspend fun <T> descriptorCall(descriptor: FileDescriptor, operation: (FileDescriptor, CancellationSignal) -> T): T {
        // Duplicate before suspension. Exactly one side claims it: the worker
        // before entering a syscall, or caller cleanup while still queued.
        // Closing the caller's original never races with an in-flight syscall
        // on a reused FD number. A stuck worker cannot write into app scratch.
        val queued = AtomicReference<ParcelFileDescriptor?>(ParcelFileDescriptor.dup(descriptor))
        try {
            return call { signal ->
                val owned = queued.getAndSet(null) ?: throw InterruptedIOException("Provider operation cancelled")
                owned.use { checkActive(signal); operation(it.fileDescriptor, signal) }
            }
        } finally {
            queued.getAndSet(null)?.close()
        }
    }

    private fun checkActive(signal: CancellationSignal) {
        if (signal.isCanceled || Thread.currentThread().isInterrupted) throw InterruptedIOException("Provider operation cancelled")
    }

    private fun waitReady(descriptor: FileDescriptor, events: Int, signal: CancellationSignal) {
        val poll = StructPollfd().apply { fd = descriptor; this.events = events.toShort() }
        while (true) {
            checkActive(signal)
            try {
                if (Os.poll(arrayOf(poll), 100) > 0) {
                    if (poll.revents.toInt() and OsConstants.POLLNVAL != 0) throw IOException("Image provider closed its descriptor")
                    return // read sees EOF on HUP; write reports EPIPE on HUP/ERR.
                }
            } catch (error: ErrnoException) { if (error.errno != OsConstants.EINTR) throw error }
        }
    }

    companion object {
        private const val CHUNK_LIMIT = 64 * 1024
        private val workers = ThreadPoolExecutor(2, 2, 0, TimeUnit.MILLISECONDS, ArrayBlockingQueue(8),
            { runnable -> Thread(runnable, "image-provider-call").apply { isDaemon = true } }, ThreadPoolExecutor.AbortPolicy())
        private val cancellations = ThreadPoolExecutor(1, 1, 0, TimeUnit.MILLISECONDS, ArrayBlockingQueue(8),
            { runnable -> Thread(runnable, "image-provider-cancel").apply { isDaemon = true } }, ThreadPoolExecutor.AbortPolicy())
    }
}
