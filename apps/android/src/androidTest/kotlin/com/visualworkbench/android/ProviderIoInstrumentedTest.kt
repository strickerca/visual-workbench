package com.visualworkbench.android

import android.content.ContentProvider
import android.content.ContentResolver
import android.content.ContentValues
import android.content.pm.ProviderInfo
import android.database.Cursor
import android.net.Uri
import android.os.CancellationSignal
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.os.ProxyFileDescriptorCallback
import android.os.SystemClock
import android.os.storage.StorageManager
import android.system.Os
import android.system.OsConstants
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.ProviderIo
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

@RunWith(AndroidJUnit4::class)
class ProviderIoInstrumentedTest {
    @Test fun nativeNonblockingFlagsPreserveDescriptorOwnership() {
        // The IN2019 run uses API 30. Its public Java inspector independently
        // checks the libc implementation used on every supported app version.
        if (Build.VERSION.SDK_INT >= 30) {
            val context = InstrumentationRegistry.getInstrumentation().targetContext
            val pipe = ParcelFileDescriptor.createPipe()
            try {
                val before = Os.fcntlInt(pipe[0].fileDescriptor, OsConstants.F_GETFL, 0)
                val fd = pipe[0].fd
                ProviderIo(context.contentResolver).nonBlocking(pipe[0])
                val after = Os.fcntlInt(pipe[0].fileDescriptor, OsConstants.F_GETFL, 0)
                assertEquals(before or OsConstants.O_NONBLOCK, after)
                assertEquals(fd, pipe[0].fd)
                assertTrue(pipe[0].fileDescriptor.valid())
            } finally { try { pipe[0].close() } finally { pipe[1].close() } }
        } else {
            throw AssertionError("This physical inspection fixture requires the authorized API 30 device")
        }
    }

    @Test fun cancelledProxyReadRetainsItsOwnDescriptorAndNeverMutatesCallerBuffer() = runBlocking {
        blockedProxy(write = false)
    }

    @Test fun cancelledProxyWriteSettlesBeforeProviderReturnsAndReleasesItsDuplicate() = runBlocking {
        blockedProxy(write = true)
    }

    private suspend fun blockedProxy(write: Boolean) = coroutineScope {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val storage = context.getSystemService(StorageManager::class.java)
        val thread = HandlerThread("synthetic-proxy-provider").also { it.start() }
        val entered = CompletableDeferred<Unit>(); val released = CompletableDeferred<Unit>()
        val received = CompletableDeferred<ByteArray>()
        val unblock = CountDownLatch(1)
        val bytes = byteArrayOf(11, 12, 13, 14)
        val callback = object : ProxyFileDescriptorCallback() {
            override fun onGetSize() = 4L
            private fun waitForFixture() {
                entered.complete(Unit)
                val deadline = SystemClock.uptimeMillis() + 5_000
                while (unblock.count > 0 && SystemClock.uptimeMillis() < deadline) {
                    try { unblock.await(50, TimeUnit.MILLISECONDS) } catch (_: InterruptedException) { }
                }
            }
            override fun onRead(offset: Long, size: Int, data: ByteArray): Int {
                check(!write); waitForFixture()
                val count = minOf(size, (4L - offset).coerceAtLeast(0).toInt())
                data.fill(77, 0, count)
                return count
            }
            override fun onWrite(offset: Long, size: Int, data: ByteArray): Int {
                check(write); waitForFixture()
                received.complete(data.copyOf(size))
                return size
            }
            override fun onFsync() = Unit
            override fun onRelease() { released.complete(Unit) }
        }
        var descriptor: ParcelFileDescriptor? = null
        var job: Job? = null
        try {
            val owned = storage.openProxyFileDescriptor(
                if (write) ParcelFileDescriptor.MODE_WRITE_ONLY else ParcelFileDescriptor.MODE_READ_ONLY,
                callback, Handler(thread.looper))
            descriptor = owned
            val io = ProviderIo(context.contentResolver)
            io.nonBlocking(owned)
            val running = launch {
                if (write) io.write(owned.fileDescriptor, bytes, bytes.size) else io.read(owned.fileDescriptor, bytes)
                error("Cancelled proxy operation delivered its result")
            }
            job = running
            withTimeout(5_000) { entered.await() }
            withTimeout(2_000) { running.cancelAndJoin() }
            owned.close(); descriptor = null
            assertFalse("The worker duplicate must remain owned until syscall return", released.isCompleted)
            bytes.fill(42)
            unblock.countDown()
            withTimeout(5_000) { released.await() }
            if (write) assertArrayEquals(byteArrayOf(11, 12, 13, 14), received.await())
            assertArrayEquals("A late read must never touch the caller's buffer", ByteArray(4) { 42 }, bytes)
        } finally {
            unblock.countDown(); job?.cancelAndJoin(); descriptor?.close()
            if (entered.isCompleted) withTimeout(5_000) { released.await() }
            thread.quitSafely(); withContext(Dispatchers.IO) { thread.join(2_000) }
            assertFalse("Proxy fixture thread must settle", thread.isAlive)
        }
    }

    @Test fun cancelledOpenDoesNotWaitForIgnoringProviderAndClosesItsLateDescriptor() = runBlocking {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val file = File(context.cacheDir.canonicalFile, "provider-fixture-${UUID.randomUUID()}").also { it.writeText("synthetic") }
        val entered = CompletableDeferred<Unit>(); val returned = CompletableDeferred<ParcelFileDescriptor>()
        val release = CountDownLatch(1)
        val provider = object : ContentProvider() {
            override fun onCreate() = true
            override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? = null
            override fun getType(uri: Uri) = "image/png"
            override fun insert(uri: Uri, values: ContentValues?): Uri? = null
            override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?) = 0
            override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = 0
            override fun openFile(uri: Uri, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
                entered.complete(Unit)
                val deadline = SystemClock.uptimeMillis() + 5_000
                while (release.count > 0 && SystemClock.uptimeMillis() < deadline) {
                    // Deliberately ignores both signal and interruption, but the
                    // fixture itself always has a bounded escape and cleanup.
                    try { release.await(50, TimeUnit.MILLISECONDS) } catch (_: InterruptedException) { }
                }
                return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).also { returned.complete(it) }
            }
        }
        provider.attachInfo(context, ProviderInfo().apply { authority = "synthetic.blocked.provider"; exported = false })
        val io = ProviderIo(ContentResolver.wrap(provider))
        val job = launch { io.open(Uri.parse("content://synthetic.blocked.provider/image"), "r").use { error("Canceled caller received a late descriptor") } }
        try {
            withTimeout(5_000) { entered.await() }
            withTimeout(2_000) { job.cancelAndJoin() }
            assertTrue(job.isCancelled); assertFalse(returned.isCompleted)
            release.countDown()
            val late = withTimeout(5_000) { returned.await() }
            withTimeout(5_000) { while (runCatching { late.fd }.isSuccess) delay(10) }
        } finally {
            release.countDown(); job.cancelAndJoin()
            if (entered.isCompleted) {
                val late = withTimeout(5_000) { returned.await() }
                withTimeout(5_000) { while (runCatching { late.fd }.isSuccess) delay(10) }
            }
            assertEquals(context.cacheDir.canonicalFile, file.canonicalFile.parentFile); assertTrue(file.delete())
        }
    }
}
