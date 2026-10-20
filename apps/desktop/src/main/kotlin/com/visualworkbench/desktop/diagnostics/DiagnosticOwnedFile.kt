package com.visualworkbench.desktop.diagnostics

import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Platform
import com.sun.jna.Pointer
import com.sun.jna.WString
import com.sun.jna.win32.StdCallLibrary
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.NoSuchFileException
import java.nio.file.Path
import java.nio.file.StandardOpenOption.CREATE_NEW
import java.nio.file.StandardOpenOption.WRITE
import java.nio.file.attribute.BasicFileAttributes

/** Creation returns the actual writer before identity acquisition can fail.
 * An unavailable identity never authorizes publication or pathname cleanup. */
internal interface DiagnosticOwnedFile {
    fun captureIdentity()
    fun write(buffer: ByteBuffer): Int
    fun force()
    fun finishWriting()
    fun publish(expectedSize: Long, action: () -> Unit)
    fun cleanup(): Boolean
}

internal fun createDiagnosticOwnedFile(path: Path): DiagnosticOwnedFile =
    if (Platform.isWindows()) WindowsDiagnosticOwnedFile.create(path)
    else NioDiagnosticOwnedFile(path, FileChannel.open(path, CREATE_NEW, WRITE, NOFOLLOW_LINKS))

private class NioDiagnosticOwnedFile(private val path: Path, private val channel: FileChannel) : DiagnosticOwnedFile {
    private var key: Any? = null
    override fun captureIdentity() {
        val attributes = attributes()
        key = checkNotNull(attributes.fileKey())
        check(attributes.isRegularFile && attributes.size() == 0L)
    }
    override fun write(buffer: ByteBuffer): Int = channel.write(buffer)
    override fun force() = channel.force(true)
    override fun finishWriting() { channel.close(); check(!channel.isOpen) }
    override fun publish(expectedSize: Long, action: () -> Unit) {
        val current = attributes()
        check(key != null && current.isRegularFile && current.fileKey() == key && current.size() == expectedSize)
        action()
    }
    override fun cleanup(): Boolean {
        finishWriting()
        if (key == null) return false
        val current = try { attributes() } catch (_: NoSuchFileException) { return true }
        if (!current.isRegularFile || current.fileKey() != key) return false
        Files.delete(path)
        return true
    }
    private fun attributes(): BasicFileAttributes = Files.readAttributes(path, BasicFileAttributes::class.java, NOFOLLOW_LINKS)
}

internal interface DiagnosticFileKernel : StdCallLibrary {
    fun CreateFileW(path: WString, access: Int, share: Int, security: Pointer?, disposition: Int, flags: Int, template: Pointer?): Pointer?
    fun GetFileInformationByHandleEx(handle: Pointer, kind: Int, buffer: Pointer, bytes: Int): Boolean
    fun WriteFile(handle: Pointer, bytes: ByteArray, count: Int, written: IntArray, overlapped: Pointer?): Boolean
    fun FlushFileBuffers(handle: Pointer): Boolean
    fun SetFileInformationByHandle(handle: Pointer, kind: Int, buffer: Pointer, bytes: Int): Boolean
    fun CloseHandle(handle: Pointer): Boolean
}

/** Windows JDK21 can return null for BasicFileAttributes.fileKey(). The native
 * CREATE_NEW handle supplies volume + 128-bit file ID; timestamps/size/content
 * are never used as a substitute for identity. No reflective JDK handle access.
 */
internal class WindowsDiagnosticOwnedFile private constructor(
    private val path: Path,
    private val api: DiagnosticFileKernel,
    private var writer: Pointer?,
) : DiagnosticOwnedFile {
    private var identity: ByteArray? = null
    private val pins = mutableListOf<Pointer>()
    private var deletionScheduled = false
    private var writingFinished = false

    override fun captureIdentity() {
        val handle = checkNotNull(writer)
        val current = inspect(handle)
        check(current.size == 0L)
        identity = current.identity
    }

    override fun write(buffer: ByteBuffer): Int {
        check(!writingFinished)
        val bytes = ByteArray(buffer.remaining())
        buffer.duplicate().get(bytes)
        val written = intArrayOf(0)
        check(api.WriteFile(checkNotNull(writer), bytes, bytes.size, written, null))
        check(written[0] in 1..bytes.size)
        buffer.position(buffer.position() + written[0])
        return written[0]
    }

    override fun force() { check(api.FlushFileBuffers(checkNotNull(writer))) }

    override fun finishWriting() { check(writer != null); writingFinished = true }

    private fun closeWriter() {
        writer?.let { handle ->
            check(api.CloseHandle(handle))
            writer = null // A failed native close retains the actual owner.
        }
    }

    override fun publish(expectedSize: Long, action: () -> Unit) {
        check(writingFinished && writer != null && identity != null && pins.isEmpty() && !deletionScheduled)
        // The original writer denies competing writes. This read pin shares
        // our writer but denies rename/delete through check + hard-link creation.
        val pin = checkNotNull(open(0x80000000.toInt(), 3))
        pins += pin
        try {
            val current = inspect(pin)
            check(sameIdentity(current) && current.size == expectedSize)
            action()
        } finally { closePins() }
    }

    override fun cleanup(): Boolean {
        if (!closePins() || identity == null) return false
        if (deletionScheduled) { closeWriter(); return true }
        // DELETE + READ_ATTRIBUTES, SHARE_READ|SHARE_WRITE. Deny competing
        // rename/delete; disposition affects this exact opened link, never a
        // pathname re-opened after the identity check.
        val pin = open(0x10000 or 0x80, 3) ?: return false
        pins += pin
        try {
            if (!sameIdentity(inspect(pin))) return false
            Memory(1).use { flag ->
                flag.setByte(0, 1)
                check(api.SetFileInformationByHandle(pin, 4, flag, 1))
            }
            deletionScheduled = true
        } finally { closePins() }
        if (pins.isNotEmpty()) return false
        closeWriter()
        return true
    }

    private data class State(val identity: ByteArray, val size: Long)
    private fun inspect(handle: Pointer): State {
        Memory(8).use { tag ->
            check(api.GetFileInformationByHandleEx(handle, 9, tag, 8))
            check(tag.getInt(0) and (0x400 or 0x10) == 0) // Neither reparse nor directory.
        }
        val size = Memory(24).use { standard ->
            check(api.GetFileInformationByHandleEx(handle, 1, standard, 24))
            check(standard.getByte(20).toInt() == 0 && standard.getByte(21).toInt() == 0)
            standard.getLong(8).also { check(it >= 0) }
        }
        val id = Memory(24).use { info ->
            check(api.GetFileInformationByHandleEx(handle, 18, info, 24))
            info.getByteArray(0, 24) // ULONGLONG volume + FILE_ID_128.
        }
        return State(id, size)
    }
    private fun sameIdentity(current: State): Boolean = identity?.contentEquals(current.identity) == true

    /** An absent name does not prove the retained original was not renamed. */
    private fun open(access: Int, share: Int): Pointer? {
        val raw = api.CreateFileW(WString(path.toString()), access, share, null, 3, 0x00200000, null)
        val nativeError = Native.getLastError()
        if (valid(raw)) return raw
        if (nativeError == 2 || nativeError == 3) return null
        error("Diagnostic file identity is unavailable.")
    }
    private fun closePins(): Boolean {
        val iterator = pins.iterator()
        while (iterator.hasNext()) if (api.CloseHandle(iterator.next())) iterator.remove()
        return pins.isEmpty()
    }

    companion object {
        fun create(path: Path, api: DiagnosticFileKernel = Native.load("kernel32", DiagnosticFileKernel::class.java)): WindowsDiagnosticOwnedFile {
            // GENERIC_WRITE + READ_ATTRIBUTES, SHARE_READ|SHARE_DELETE. Keep
            // this actual handle through uncertain cleanup, preventing ID reuse
            // and competing writes while allowing a replacement to be detected.
            val raw = api.CreateFileW(WString(path.toString()), 0x40000000 or 0x80, 5, null, 1, 0x00200000, null)
            check(valid(raw))
            return WindowsDiagnosticOwnedFile(path, api, raw)
        }
        private fun valid(handle: Pointer?): Boolean = handle != null && Pointer.nativeValue(handle) != -1L
    }
}
