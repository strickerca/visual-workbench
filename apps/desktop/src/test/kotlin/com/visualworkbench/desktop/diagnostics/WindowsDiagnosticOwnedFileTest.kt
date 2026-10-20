package com.visualworkbench.desktop.diagnostics

import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.WString
import java.nio.ByteBuffer
import java.nio.file.Path
import org.junit.Assert.*
import org.junit.Test

/** Native-call contract fixtures, not Windows/filesystem execution evidence. */
class WindowsDiagnosticOwnedFileTest {
    private class Kernel : DiagnosticFileKernel {
        data class Open(val access: Int, val share: Int, val disposition: Int, val flags: Int)
        val opens = mutableListOf<Open>()
        val handles = mutableMapOf<Pointer, ByteArray>()
        val deleted = mutableListOf<ByteArray>()
        var pathIdentity = ByteArray(24).apply { this[0] = 1; this[8] = 2 }
        var attributes = 0x80
        var size = 0L
        var identityAvailable = true
        var openError = 0
        var refuseCloses = false
        var refuseDisposition = false
        var flushed = false
        var next = 1L
        val written = mutableListOf<Byte>()
        override fun CreateFileW(path: WString, access: Int, share: Int, security: Pointer?, disposition: Int, flags: Int, template: Pointer?): Pointer? {
            opens += Open(access, share, disposition, flags)
            if (openError != 0) { Native.setLastError(openError); return Pointer(-1L) }
            return Pointer(next++).also { handles[it] = pathIdentity.copyOf() }
        }
        override fun GetFileInformationByHandleEx(handle: Pointer, kind: Int, buffer: Pointer, bytes: Int): Boolean {
            check(handle in handles)
            buffer.clear(bytes.toLong())
            when (kind) {
                9 -> { assertEquals(8, bytes); buffer.setInt(0, attributes) }
                1 -> { assertEquals(24, bytes); buffer.setLong(8, size) }
                18 -> {
                    assertEquals(24, bytes)
                    if (!identityAvailable) return false
                    buffer.write(0, checkNotNull(handles[handle]), 0, 24)
                }
                else -> error("Unexpected file information class")
            }
            return true
        }
        override fun WriteFile(handle: Pointer, bytes: ByteArray, count: Int, written: IntArray, overlapped: Pointer?): Boolean {
            check(handle in handles)
            // Short native writes must advance only the actual accepted bytes.
            written[0] = minOf(2, count)
            this.written.addAll(bytes.take(written[0]))
            size += written[0]
            return true
        }
        override fun FlushFileBuffers(handle: Pointer): Boolean { check(handle in handles); flushed = true; return true }
        override fun SetFileInformationByHandle(handle: Pointer, kind: Int, buffer: Pointer, bytes: Int): Boolean {
            assertEquals(4, kind); assertEquals(1, bytes); assertEquals(1, buffer.getByte(0).toInt())
            if (refuseDisposition) return false
            deleted += checkNotNull(handles[handle]).copyOf()
            return true
        }
        override fun CloseHandle(handle: Pointer): Boolean {
            check(handle in handles)
            if (refuseCloses) return false
            handles.remove(handle)
            return true
        }
    }

    private fun create(kernel: Kernel) = WindowsDiagnosticOwnedFile.create(Path.of("owned.tmp"), kernel)
    private fun prepared(kernel: Kernel): WindowsDiagnosticOwnedFile = create(kernel).also { file ->
        file.captureIdentity()
        val bytes = ByteBuffer.wrap(byteArrayOf(10, 20, 30))
        while (bytes.hasRemaining()) file.write(bytes)
        file.force()
        file.finishWriting()
    }
    private fun refused(action: () -> Unit) { assertTrue(runCatching(action).isFailure) }

    @Test fun nativeWriterPinsPublicationAndDeletesOnlyItsCapturedIdentity() {
        val kernel = Kernel()
        val original = kernel.pathIdentity.copyOf()
        val file = prepared(kernel)
        var published = false
        file.publish(3) {
            assertEquals(2, kernel.handles.size)
            assertEquals(Int.MIN_VALUE, kernel.opens.last().access)
            assertEquals(3, kernel.opens.last().share)
            published = true
        }
        assertTrue(published); assertTrue(kernel.flushed)
        assertEquals(listOf<Byte>(10, 20, 30), kernel.written)
        assertEquals(1, kernel.opens.first().disposition)
        assertEquals(5, kernel.opens.first().share)
        assertTrue(kernel.opens.all { it.flags == 0x00200000 })
        assertTrue(file.cleanup())
        assertEquals(0x10080, kernel.opens.last().access)
        assertEquals(3, kernel.opens.last().share)
        assertEquals(1, kernel.deleted.size)
        assertArrayEquals(original, kernel.deleted.single())
        assertTrue(kernel.handles.isEmpty())
    }

    @Test fun volumeAndFull128BitIdBothRejectSameSizeReplacements() {
        for (changedByte in listOf(0, 7, 8, 23)) {
            val kernel = Kernel()
            val file = prepared(kernel)
            kernel.pathIdentity[changedByte] = (kernel.pathIdentity[changedByte] + 1).toByte()
            var published = false
            refused { file.publish(3) { published = true } }
            assertFalse(published)
            assertFalse(file.cleanup())
            assertTrue(kernel.deleted.isEmpty())
            assertEquals(1, kernel.handles.size) // Actual original identity remains held.
        }
    }

    @Test fun unknownIdentityRetainsCleanupWithoutNullOrMetadataFallback() {
        val kernel = Kernel().apply { identityAvailable = false }
        val file = create(kernel)
        refused { file.captureIdentity() }
        file.finishWriting()
        refused { file.publish(0) { fail("Unknown identity published") } }
        assertFalse(file.cleanup())
        assertEquals(1, kernel.opens.size)
        assertTrue(kernel.deleted.isEmpty())
        assertEquals(1, kernel.handles.size)
    }

    @Test fun reparseOrDirectoryAtPublicationCannotPublishOrDeleteForeignFile() {
        for (flag in listOf(0x400, 0x10)) {
            val kernel = Kernel()
            val file = prepared(kernel)
            kernel.attributes = flag
            refused { file.publish(3) { fail("Unexpected file type published") } }
            refused { file.cleanup() }
            assertTrue(kernel.deleted.isEmpty())
            assertEquals(1, kernel.handles.size)
        }
    }

    @Test fun sizeMismatchCannotPublishEvenWithExactIdentity() {
        val kernel = Kernel()
        val file = prepared(kernel)
        refused { file.publish(4) { fail("Wrong size published") } }
        assertTrue(file.cleanup())
    }

    @Test fun unfinishedWriterCannotPublishAndFailedCleanupClosesRetainOwners() {
        val kernel = Kernel()
        val file = create(kernel)
        file.captureIdentity()
        kernel.refuseCloses = true
        refused { file.publish(0) { fail("Unfinished writer published") } }
        assertEquals(1, kernel.handles.size)
        assertFalse(file.cleanup())
        assertEquals(2, kernel.handles.size)
        assertEquals(1, kernel.deleted.size)
        kernel.refuseCloses = false
        assertTrue(file.cleanup())
        assertTrue(kernel.handles.isEmpty())
    }

    @Test fun deletionCloseFailureRetainsHandleAndRetryCannotTouchReplacement() {
        val kernel = Kernel()
        val file = prepared(kernel)
        kernel.refuseCloses = true
        assertFalse(file.cleanup())
        assertEquals(2, kernel.handles.size)
        assertEquals(1, kernel.deleted.size)
        val openCount = kernel.opens.size
        kernel.pathIdentity[23] = 9
        kernel.refuseCloses = false
        assertTrue(file.cleanup())
        assertEquals(openCount, kernel.opens.size)
        assertEquals(1, kernel.deleted.size)
        assertTrue(kernel.handles.isEmpty())
    }

    @Test fun refusedDispositionRetainsOwnershipForAnExplicitRetry() {
        val kernel = Kernel()
        val file = prepared(kernel)
        kernel.refuseDisposition = true
        refused { file.cleanup() }
        assertTrue(kernel.deleted.isEmpty())
        kernel.refuseDisposition = false
        assertTrue(file.cleanup())
        assertEquals(1, kernel.deleted.size)
    }

    @Test fun absentSiblingRetainsOriginalAndOtherPathErrorsAlsoCannotDelete() {
        for (code in listOf(2, 3, 5, 32)) {
            val kernel = Kernel()
            val file = prepared(kernel)
            kernel.openError = code
            if (code in listOf(2, 3)) assertFalse(file.cleanup()) else refused { file.cleanup() }
            assertTrue(kernel.deleted.isEmpty())
            assertEquals(1, kernel.handles.size)
        }
    }

    @Test fun failedPublicationPinCloseRetainsBothOwnersUntilActualRetirement() {
        val kernel = Kernel()
        val file = prepared(kernel)
        kernel.refuseCloses = true
        var published = false
        file.publish(3) { published = true }
        assertTrue(published)
        assertEquals(2, kernel.handles.size)
        assertFalse(file.cleanup())
        assertTrue(kernel.deleted.isEmpty())
        kernel.refuseCloses = false
        assertTrue(file.cleanup())
        assertEquals(1, kernel.deleted.size)
        assertTrue(kernel.handles.isEmpty())
    }
}
