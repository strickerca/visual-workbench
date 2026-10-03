package com.visualworkbench.desktop

import java.nio.file.FileAlreadyExistsException
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicBoolean

/** This is the sole exception to exact opened-path equality. It applies only
 * to the fixed application-directory anchor, never to a runtime file. The
 * production guard checks directory/reparse attributes on the opened handle. */
internal interface NativeAnchorGuard : NativeFileGuard {
    fun pinAnchor(path: Path): NativeDirectoryLease
}
internal data class NativeDirectoryLease(val path: Path, val lease: AutoCloseable, val verify: () -> Unit)

/** Both the original app-data chain and the OS-resolved anchor stay pinned
 * while prepareAt independently pins the complete physical runtime chain. */
internal class NativeCacheAnchor private constructor(
    val directory: Path,
    private val leases: List<AutoCloseable>,
    private val verifyHandle: () -> Unit,
) : AutoCloseable {
    private val closed = AtomicBoolean(false)
    override fun close() {
        if (closed.compareAndSet(false, true)) leases.asReversed().forEach { runCatching { it.close() } }
    }
    /** Recheck after the physical ancestor pins are acquired. A namespace
     * rename during the original-to-physical handoff must never be admitted. */
    fun verify() { check(!closed.get()); verifyHandle() }

    companion object {
        private const val NAME = "Visual Workbench"

        fun prepare(localAppData: Path, guard: NativeAnchorGuard): NativeCacheAnchor {
            val pins = mutableListOf<AutoCloseable>()
            try {
                if (!localAppData.isAbsolute)
                    throw NativeRuntimeFailure("The native cache requires an absolute app-data root.")
                val original = localAppData.toAbsolutePath().normalize()
                if (original.nameCount > 256 || original.toString().length > 32767)
                    throw NativeRuntimeFailure("The native cache ancestor chain exceeds its limit.")
                // LOCALAPPDATA itself and every existing ancestor must still
                // resolve exactly. Missing ancestors are never created here.
                val chain = generateSequence(original) { it.parent }.toList().asReversed()
                for (part in chain) {
                    guard.check(part)
                    if (!Files.isDirectory(part, LinkOption.NOFOLLOW_LINKS))
                        throw NativeRuntimeFailure("The native cache app-data ancestor is unavailable.")
                    pins += guard.pinDirectory(part)
                    if (part.toRealPath() != part)
                        throw NativeRuntimeFailure("The native cache app-data ancestor is redirected.")
                }

                // Only this one immediate child may be interpreted by the OS
                // app-data namespace. There is no package-name inference and
                // no arbitrary path/URL supplied by project content.
                val opened = original.resolve(NAME)
                if (!Files.exists(opened, LinkOption.NOFOLLOW_LINKS)) {
                    try { Files.createDirectory(opened) } catch (_: FileAlreadyExistsException) { }
                }
                guard.check(opened)
                val resolved = guard.pinAnchor(opened)
                pins += resolved.lease
                val canonical = resolved.path
                if (!canonical.isAbsolute || canonical.nameCount > 256 || canonical.toString().length > 32767 || canonical != canonical.normalize() ||
                    canonical == original || !canonical.startsWith(original) ||
                    !canonical.fileName.toString().equals(NAME, ignoreCase = true))
                    throw NativeRuntimeFailure("The native cache anchor resolved outside its admitted app-data root.")
                return NativeCacheAnchor(canonical, pins.toList(), resolved.verify)
            } catch (error: Exception) {
                pins.asReversed().forEach { runCatching { it.close() } }
                // No runtime file has been created yet. A newly created empty
                // anchor is preserved; we never guess which virtual path to delete.
                if (error is NativeRuntimeFailure) throw error
                throw NativeRuntimeFailure("The native cache anchor could not be pinned safely.")
            }
        }
    }
}

/** GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED | VOLUME_NAME_DOS, returns
 * one bounded \\?\ drive path. UNC/device/relative/ADS/dot paths are refused.
 * This only parses the handle result; it never accepts a caller's substitute. */
internal fun nativeCanonicalDosPath(value: String): Path {
    if (value.length !in 7..32767 || !value.startsWith("\\\\?\\"))
        throw NativeRuntimeFailure("The native cache handle did not return a bounded DOS path.")
    val dos = value.substring(4)
    if (dos.length < 3 || !dos[0].isAsciiDrive() || dos[1] != ':' || dos[2] != '\\' ||
        dos.drop(2).any { it == ':' || it == '/' || it == '\u0000' })
        throw NativeRuntimeFailure("The native cache handle returned an unsupported path namespace.")
    val path = try { Path.of(dos) } catch (_: Exception) {
        throw NativeRuntimeFailure("The native cache handle returned an invalid path.")
    }
    if (!path.isAbsolute || path != path.normalize() || !path.toString().equals(dos, ignoreCase = true))
        throw NativeRuntimeFailure("The native cache handle returned a noncanonical path.")
    return path
}
private fun Char.isAsciiDrive(): Boolean = this in 'A'..'Z' || this in 'a'..'z'
