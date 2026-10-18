package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.CoreFailure
import com.visualworkbench.shared.CoreFailureKind
import com.visualworkbench.shared.PackageInfo
import com.visualworkbench.shared.WorkbenchCore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.Files
import java.nio.file.LinkOption.NOFOLLOW_LINKS
import java.nio.file.NoSuchFileException
import java.nio.file.Path
import java.nio.file.StandardOpenOption.READ
import java.nio.file.attribute.BasicFileAttributes

private const val RECOVERED_CAPTURE_LIMIT = 32
private const val RECOVERED_ROOT_COMPONENT_LIMIT = 64
private val recoveredCaptureName = Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
private val recoveredCaptureOwner = "Visual Workbench private MCP capture v1\n".toByteArray(Charsets.UTF_8)

/** Finds an existing canonical capture project, not a new source owner/cache.
 * Only the existing engine supplies its app-private mcp-captures-v1 root while
 * serializing its normal source-map/capture lifecycle. No package directory,
 * public lineage, image bytes, OS selector or persisted grant supplies a path.
 *
 * Admit the entire <=32 direct-child inventory before opening any project.
 * UUID workdirs require the exact bounded private owner marker; missing project
 * directories are normal interrupted captures. Foreign entries, malformed
 * markers, links and noncanonical paths refuse. Ancestry is bounded to 64 root
 * components plus two fixed descendants. No recursive traversal occurs.
 *
 * Open one existing WorkbenchProject at a time and close it NonCancellable
 * before continuing/returning. Only Invalid/Storage/Unsupported failures from
 * an unknown candidate's open are skippable; info/close, lifecycle, capacity and
 * worker failures propagate. A failed close never becomes 'not found'. The
 * returned path is merely a hint for the engine's existing source map: that
 * owner reopens it and native verification checks source asset/capture binding.
 */
internal suspend fun findRetainedMcpCaptureProject(
    root: Path, expected: PackageInfo, core: WorkbenchCore, excluded: Set<Path> = emptySet(),
): Path? {
    currentCoroutineContext().ensureActive()
    require(root.isAbsolute && root == root.normalize() && root.fileName?.toString() == "mcp-captures-v1") {
        "Invalid retained capture root"
    }
    require(root.nameCount <= RECOVERED_ROOT_COMPONENT_LIMIT && excluded.size <= RECOVERED_CAPTURE_LIMIT) {
        "Retained capture recovery limit"
    }
    val candidates = withContext(Dispatchers.IO) { recoveredCaptureCandidates(root) }
    for (candidate in candidates) {
        currentCoroutineContext().ensureActive()
        if (candidate in excluded) continue
        // The existing engine gate excludes its own concurrent directory/source
        // changes. Recheck paths/owner immediately before borrowing a candidate;
        // the native project opener remains responsible for project internals.
        val stillPresent = withContext(Dispatchers.IO) { checkedRecoveredProject(root, candidate.parent) }
        if (stillPresent == null) continue
        val project = try {
            core.open(candidate.toString())
        } catch (error: CoreFailure) {
            when (error.kind) {
                CoreFailureKind.Invalid, CoreFailureKind.Storage, CoreFailureKind.Unsupported -> {
                    currentCoroutineContext().ensureActive()
                    continue
                }
                else -> throw error
            }
        }
        val matches = try {
            currentCoroutineContext().ensureActive()
            val info = project.info()
            currentCoroutineContext().ensureActive()
            info.projectId == expected.binding.projectId && expected.binding.documentId in info.documentIds &&
                info.hostSeq == expected.binding.hostSeq && info.stateHash == expected.binding.stateHash
        } finally {
            // Preserve the actual close failure even during cancellation. An
            // uncertain owner must stop the scan rather than open a second one.
            withContext(NonCancellable) { project.close() }
        }
        currentCoroutineContext().ensureActive()
        if (matches) {
            val retained = withContext(Dispatchers.IO) { checkedRecoveredProject(root, candidate.parent) }
            currentCoroutineContext().ensureActive()
            return retained
        }
    }
    return null
}

private suspend fun recoveredCaptureCandidates(root: Path): List<Path> {
    val attributes = recoveredCanonicalAttributes(root) ?: return emptyList()
    require(attributes.isDirectory) { "Invalid retained capture root" }
    val workdirs = ArrayList<Path>(RECOVERED_CAPTURE_LIMIT)
    Files.newDirectoryStream(root).use { children ->
        for (child in children) {
            currentCoroutineContext().ensureActive()
            require(workdirs.size < RECOVERED_CAPTURE_LIMIT) { "Retained capture recovery limit" }
            workdirs.add(child)
        }
    }
    workdirs.sortBy { it.fileName.toString() }
    val projects = ArrayList<Path>(workdirs.size)
    for (workdir in workdirs) {
        currentCoroutineContext().ensureActive()
        checkedRecoveredProject(root, workdir)?.let { projects.add(it) }
    }
    return projects
}

private fun checkedRecoveredProject(root: Path, workdir: Path): Path? {
    require(workdir.parent == root && recoveredCaptureName.matches(workdir.fileName.toString())) {
        "Foreign retained capture entry"
    }
    val work = recoveredCanonicalAttributes(workdir)
    require(work != null && work.isDirectory) { "Invalid retained capture workdir" }
    val marker = workdir.resolve("owner-v1")
    val owner = recoveredCanonicalAttributes(marker)
    require(owner != null && owner.isRegularFile && owner.size() == recoveredCaptureOwner.size.toLong()) {
        "Invalid retained capture owner"
    }
    // Size/type/path admission precedes this fixed 41-byte allocation/read
    // (40 exact marker bytes plus one byte to detect a growing file).
    // NOFOLLOW_LINKS also refuses a marker changed to a link after metadata IO.
    val bytes = ByteArray(recoveredCaptureOwner.size + 1)
    val buffer = ByteBuffer.wrap(bytes)
    FileChannel.open(marker, READ, NOFOLLOW_LINKS).use { channel ->
        require(channel.size() == recoveredCaptureOwner.size.toLong()) { "Invalid retained capture owner" }
        while (buffer.hasRemaining()) {
            val read = channel.read(buffer)
            if (read < 0) break
            require(read > 0) { "Retained capture owner read stalled" }
        }
        require(buffer.position() == recoveredCaptureOwner.size && channel.size() == recoveredCaptureOwner.size.toLong()) {
            "Invalid retained capture owner"
        }
    }
    for (index in recoveredCaptureOwner.indices) require(bytes[index] == recoveredCaptureOwner[index]) {
        "Foreign retained capture owner"
    }
    val project = workdir.resolve("project.vwb")
    val attributes = recoveredCanonicalAttributes(project) ?: return null
    require(attributes.isDirectory) { "Invalid retained capture project" }
    return project
}

/** Metadata only, including existing ancestors. A missing descendant is not
 * authority to follow a link or normalize an alternate path into this root. */
private fun recoveredCanonicalAttributes(path: Path): BasicFileAttributes? {
    require(path.isAbsolute && path == path.normalize() && path.nameCount <= RECOVERED_ROOT_COMPONENT_LIMIT + 2) {
        "Noncanonical retained capture path"
    }
    var at = path.root
    var last: BasicFileAttributes? = null
    for (part in path) {
        at = at.resolve(part)
        val attributes = try {
            Files.readAttributes(at, BasicFileAttributes::class.java, NOFOLLOW_LINKS)
        } catch (_: NoSuchFileException) {
            return null
        }
        require(!attributes.isSymbolicLink && (at == path || attributes.isDirectory) && at.toRealPath() == at) {
            "Noncanonical retained capture path"
        }
        last = attributes
    }
    return last
}
