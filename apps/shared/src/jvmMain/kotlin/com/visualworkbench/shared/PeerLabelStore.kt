package com.visualworkbench.shared

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.NoSuchFileException
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.nio.file.StandardOpenOption
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.Collections
import java.util.Properties

/** Presentation only. Names never identify a trusted peer or a network route. */
public enum class PeerLabelIssue { Unreadable, InvalidStore, ChangedStore, SaveFailed, InvalidName, InvalidPeer }

public class PeerLabelSnapshot internal constructor(
    labels: Map<String, String>,
    public val writable: Boolean,
    public val issue: PeerLabelIssue? = null,
) {
    public val labels: Map<String, String> = Collections.unmodifiableMap(labels.toMap())
    public fun nameFor(deviceId: String, fingerprint: String): String? = peerLabelKey(deviceId, fingerprint)?.let(labels::get)
    override fun toString(): String = "PeerLabelSnapshot(writable=$writable,issue=$issue)"
}

/** A trimmed single line, measured in Unicode code points rather than UTF-16 units. */
public fun checkedPeerLabel(value: String): String? {
    // Bound work before walking code points; 64 supplementary points use 128 units.
    if (value.length > 256 || !safeLabelText(value)) return null
    val label = value.trim()
    return label.takeIf { it.codePointCount(0, it.length) in 1..64 }
}

private fun safeLabelText(value: String): Boolean {
    var offset = 0
    while (offset < value.length) {
        val point = value.codePointAt(offset)
        if (Character.isISOControl(point) || point in 0xD800..0xDFFF ||
            point == 0x061C || point in 0x200E..0x200F || point in 0x2028..0x202E || point in 0x2066..0x206F) return false
        offset += Character.charCount(point)
    }
    return true
}

private fun peerLabelKey(deviceId: String, fingerprint: String): String? {
    if (deviceId.isBlank() || deviceId.length > 128 || !safeLabelText(deviceId) ||
        !fingerprint.matches(Regex("[0-9a-f]{64}"))) return null
    return digest((deviceId + '\u0000' + fingerprint).toByteArray(Charsets.UTF_8))
}

private fun digest(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes)
    .joinToString("") { (it.toInt() and 255).toString(16).padStart(2, '0') }

/** One separate, bounded local label file. Call on an IO dispatcher. Existing
 * unreadable/invalid/drifted bytes are never repaired, removed, or overwritten.
 * This class does not read preferences, protected trust, identity or projects. */
public class PeerLabelStore(file: Path) {
    private val path = file.toAbsolutePath().normalize()
    private var loaded = false
    private var originalHash: String? = null
    private var snapshot = PeerLabelSnapshot(emptyMap(), false)

    @Synchronized
    public fun load(): PeerLabelSnapshot {
        if (loaded) return snapshot
        loaded = true
        try {
            val bytes = readFile()
            originalHash = bytes?.let(::digest)
            snapshot = PeerLabelSnapshot(bytes?.let(::parse) ?: emptyMap(), true)
        } catch (_: InvalidLabels) { snapshot = blocked(PeerLabelIssue.InvalidStore) }
        catch (_: Exception) { snapshot = blocked(PeerLabelIssue.Unreadable) }
        return snapshot
    }

    @Synchronized
    public fun rename(deviceId: String, fingerprint: String, label: String): PeerLabelSnapshot {
        load()
        val key = peerLabelKey(deviceId, fingerprint)
            ?: return PeerLabelSnapshot(snapshot.labels, snapshot.writable, PeerLabelIssue.InvalidPeer)
        val checked = checkedPeerLabel(label)
            ?: return PeerLabelSnapshot(snapshot.labels, snapshot.writable, PeerLabelIssue.InvalidName)
        if (!snapshot.writable) return snapshot
        var temporary: Path? = null
        try {
            if (readFile()?.let(::digest) != originalHash) return block(PeerLabelIssue.ChangedStore)
            val labels = snapshot.labels + (key to checked)
            if (labels.size > MAX_LABELS) return PeerLabelSnapshot(snapshot.labels, true, PeerLabelIssue.SaveFailed)
            val properties = Properties().also { data ->
                data.setProperty("schema", "1")
                labels.toSortedMap().forEach { (identity, name) -> data.setProperty("peer.$identity", name) }
            }
            val bytes = ByteArrayOutputStream().also { properties.store(it, "Local presentation labels") }.toByteArray()
            if (bytes.size > MAX_BYTES) return PeerLabelSnapshot(snapshot.labels, true, PeerLabelIssue.SaveFailed)
            ordinaryPath(path)
            Files.createDirectories(checkNotNull(path.parent))
            ordinaryPath(path)
            temporary = Files.createTempFile(path.parent, ".peer-labels-", ".tmp")
            Files.newOutputStream(temporary, StandardOpenOption.WRITE, StandardOpenOption.TRUNCATE_EXISTING,
                LinkOption.NOFOLLOW_LINKS).use { it.write(bytes) }
            // Recheck immediately before the one atomic replacement; no fallback
            // non-atomic write or retry can erase unknown existing bytes.
            ordinaryPath(path)
            if (readFile()?.let(::digest) != originalHash) return block(PeerLabelIssue.ChangedStore)
            Files.move(temporary, path, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            temporary = null
            if (!checkNotNull(readFile()).contentEquals(bytes)) return block(PeerLabelIssue.SaveFailed)
            originalHash = digest(bytes)
            snapshot = PeerLabelSnapshot(labels, true)
        } catch (_: InvalidLabels) { return block(PeerLabelIssue.InvalidStore) }
        catch (_: Exception) { return block(PeerLabelIssue.SaveFailed) }
        finally {
            temporary?.let { owned ->
                // Only this invocation's exact temporary file is eligible.
                try { if (owned.parent == path.parent) { ordinaryPath(owned); Files.deleteIfExists(owned) } }
                catch (_: Exception) { /* Preserve an uncertain temporary file. */ }
            }
        }
        return snapshot
    }

    private fun block(issue: PeerLabelIssue): PeerLabelSnapshot = blocked(issue).also { snapshot = it }
    private fun blocked(issue: PeerLabelIssue): PeerLabelSnapshot = PeerLabelSnapshot(emptyMap(), false, issue)

    private fun readFile(): ByteArray? {
        ordinaryPath(path)
        val attributes = try { Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS) }
            catch (_: NoSuchFileException) { return null }
        if (!attributes.isRegularFile || attributes.size() > MAX_BYTES) throw InvalidLabels()
        return Files.newInputStream(path, LinkOption.NOFOLLOW_LINKS).use { input ->
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(4096)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (output.size() + count > MAX_BYTES) throw InvalidLabels()
                output.write(buffer, 0, count)
            }
            output.toByteArray()
        }
    }

    private fun parse(bytes: ByteArray): Map<String, String> {
        val properties = Properties()
        try { ByteArrayInputStream(bytes).use(properties::load) } catch (_: IllegalArgumentException) { throw InvalidLabels() }
        if (properties.getProperty("schema") != "1" || properties.size > MAX_LABELS + 1) throw InvalidLabels()
        val labels = linkedMapOf<String, String>()
        for (key in properties.stringPropertyNames()) {
            if (key == "schema") continue
            if (!key.matches(Regex("peer\\.[0-9a-f]{64}"))) throw InvalidLabels()
            val value = properties.getProperty(key)
            if (checkedPeerLabel(value) != value) throw InvalidLabels()
            labels[key.removePrefix("peer.")] = value
        }
        return labels
    }

    private fun ordinaryPath(target: Path) {
        var current = checkNotNull(target.root)
        for (part in target) {
            current = current.resolve(part)
            val attributes = try { Files.readAttributes(current, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS) }
                catch (_: NoSuchFileException) { continue }
            if (attributes.isSymbolicLink || attributes.isOther ||
                (current != target && !attributes.isDirectory) || current.toRealPath() != current) throw InvalidLabels()
        }
    }

    private class InvalidLabels : Exception()
    private companion object {
        const val MAX_BYTES = 65_536
        const val MAX_LABELS = 64
    }
}
