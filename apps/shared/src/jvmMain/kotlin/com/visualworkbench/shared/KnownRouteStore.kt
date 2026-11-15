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
import java.util.Properties

private const val MAX_ROUTE_FILE_BYTES = 65_536
private const val MAX_SAVED_PEERS = 64
private const val MAX_SAVED_ROUTES = 6

private const val MAX_SAVED_ADAPTERS = 8

/** What the app remembers so that paired devices connect without typing:
 * the owner's automatic-connection setting, the network adapters the owner
 * chose (which ones to listen on, and which one is USB tethering), and where
 * each paired device was last authenticated. Addresses are hints for the next
 * dial and never trust. This is app-level data like the pairing list: it
 * never goes into a project, an export, a log or a report. */
public class SavedRoutes internal constructor(
    public val automatic: Boolean,
    public val tetherAdapter: String?,
    public val lastPeer: String?,
    routes: Map<String, List<SessionEndpoint>>,
    adapters: Collection<String>,
) {
    public constructor() : this(AUTOMATIC_BY_DEFAULT, null, null, emptyMap(), emptyList())

    private val routes: Map<String, List<SessionEndpoint>> = LinkedHashMap(routes)

    /** Network adapters the owner chose for this app, by the name the system
     * gives them. The automatic listener uses only these. */
    public val adapters: Set<String> = LinkedHashSet(adapters)

    /** Keys of the devices that have saved routes. See [KnownRouteStore.peerKey]. */
    public val peers: Set<String> get() = routes.keys
    public fun routesFor(peerKey: String?): List<SessionEndpoint> = routes[peerKey].orEmpty()
    public fun isLastPeer(peerKey: String?): Boolean = peerKey != null && peerKey == lastPeer

    private fun changed(automatic: Boolean = this.automatic, tetherAdapter: String? = this.tetherAdapter, lastPeer: String? = this.lastPeer,
        routes: Map<String, List<SessionEndpoint>> = this.routes, adapters: Collection<String> = this.adapters): SavedRoutes =
        SavedRoutes(automatic, tetherAdapter, lastPeer, routes, adapters)

    public fun withAutomatic(value: Boolean): SavedRoutes = changed(automatic = value)

    /** The adapter the owner chose for USB tethering, or null to forget it.
     * A name alone is never treated as proof of a USB link. */
    public fun withTetherAdapter(name: String?): SavedRoutes = changed(tetherAdapter = name?.let(::checkedPeerLabel))

    /** An adapter the owner chose in the connection sheet. The newest eight are kept. */
    public fun withAdapter(name: String): SavedRoutes {
        val checked = checkedPeerLabel(name) ?: return this
        if (checked in adapters) return this
        return changed(adapters = (adapters + checked).toList().takeLast(MAX_SAVED_ADAPTERS))
    }

    /** Forget every chosen adapter, including the one for USB tethering. */
    public fun withoutAdapters(): SavedRoutes = changed(tetherAdapter = null, adapters = emptyList())

    /** Newest first. An empty list forgets the device's routes. */
    public fun remember(peerKey: String, endpoints: List<SessionEndpoint>): SavedRoutes {
        if (!validKey(peerKey)) return this
        val kept = endpoints.filter { isNumericSessionEndpoint(it.address) }.distinctBy { it.address }.take(MAX_SAVED_ROUTES)
        val next = LinkedHashMap(routes)
        next.remove(peerKey)
        if (kept.isNotEmpty()) {
            while (next.size >= MAX_SAVED_PEERS) next.remove(next.keys.first())
            next[peerKey] = kept
        }
        return changed(lastPeer = peerKey, routes = next)
    }

    public fun forget(peerKey: String): SavedRoutes = if (peerKey in routes) changed(routes = routes - peerKey) else this

    /** Note the device a connection was last made with, leaving routes alone. */
    public fun withLastPeer(peerKey: String): SavedRoutes = if (validKey(peerKey)) changed(lastPeer = peerKey) else this

    /** Drop everything saved for devices that are no longer paired. */
    public fun keepOnly(peerKeys: Set<String>): SavedRoutes =
        changed(lastPeer = lastPeer?.takeIf { it in peerKeys }, routes = routes.filterKeys { it in peerKeys })

    internal fun sameAs(other: SavedRoutes): Boolean = automatic == other.automatic && tetherAdapter == other.tetherAdapter &&
        lastPeer == other.lastPeer && routes == other.routes && adapters.toList() == other.adapters.toList()

    internal fun encode(): ByteArray {
        val properties = Properties()
        properties.setProperty("schema", "1")
        properties.setProperty("automatic", automatic.toString())
        tetherAdapter?.let { properties.setProperty("tether", it) }
        lastPeer?.let { properties.setProperty("last", it) }
        adapters.forEachIndexed { index, name -> properties.setProperty("adapter.$index", name) }
        for ((key, endpoints) in routes) endpoints.forEachIndexed { index, endpoint ->
            properties.setProperty("route.$key.$index", "${carrierWord(endpoint.carrier)} ${endpoint.address}")
        }
        return ByteArrayOutputStream().also { properties.store(it, "Automatic connection settings") }.toByteArray()
    }

    override fun toString(): String = "SavedRoutes([REDACTED])"

    internal companion object {
        fun validKey(key: String): Boolean = key.length == 64 && key.all { it in '0'..'9' || it in 'a'..'f' }
        private fun carrierWord(carrier: SessionCarrier): String = when (carrier) {
            SessionCarrier.QuicTether -> "usb"
            SessionCarrier.QuicWifi -> "wifi"
            SessionCarrier.TcpAdb -> "adb"
        }
        private fun index(text: String, limit: Int): Int? = text.toIntOrNull()?.takeIf { it in 0 until limit && it.toString() == text }

        /** Null for anything this version did not write itself. */
        fun decode(bytes: ByteArray): SavedRoutes? {
            val properties = Properties()
            try { ByteArrayInputStream(bytes).use(properties::load) } catch (_: IllegalArgumentException) { return null }
            if (properties.getProperty("schema") != "1" || properties.size > 4 + MAX_SAVED_ADAPTERS + MAX_SAVED_PEERS * MAX_SAVED_ROUTES) return null
            val automatic = when (properties.getProperty("automatic")) { "true" -> true; "false" -> false; else -> return null }
            val tether = properties.getProperty("tether")?.let { value -> checkedPeerLabel(value)?.takeIf { it == value } ?: return null }
            val last = properties.getProperty("last")?.also { if (!validKey(it)) return null }
            val found = HashMap<String, MutableMap<Int, SessionEndpoint>>()
            val adapters = HashMap<Int, String>()
            for (name in properties.stringPropertyNames()) {
                if (name in setOf("schema", "automatic", "tether", "last")) continue
                val parts = name.split('.')
                val value = properties.getProperty(name)
                if (parts.size == 2 && parts[0] == "adapter") {
                    adapters[index(parts[1], MAX_SAVED_ADAPTERS) ?: return null] = checkedPeerLabel(value)?.takeIf { it == value } ?: return null
                    continue
                }
                if (parts.size != 3 || parts[0] != "route" || !validKey(parts[1])) return null
                val position = index(parts[2], MAX_SAVED_ROUTES) ?: return null
                val carrier = when (value.substringBefore(' ')) {
                    "usb" -> SessionCarrier.QuicTether
                    "wifi" -> SessionCarrier.QuicWifi
                    "adb" -> SessionCarrier.TcpAdb
                    else -> return null
                }
                val address = value.substringAfter(' ', "")
                if (!isNumericSessionEndpoint(address)) return null
                found.getOrPut(parts[1]) { HashMap() }[position] = SessionEndpoint(carrier, address)
            }
            if (found.size > MAX_SAVED_PEERS) return null
            val routes = LinkedHashMap<String, List<SessionEndpoint>>()
            for (key in found.keys.sorted()) routes[key] = found.getValue(key).toSortedMap().values.toList()
            return SavedRoutes(automatic, tether, last, routes, adapters.toSortedMap().values.distinct())
        }
    }
}

/** One small app-private file. Call on an IO dispatcher. A path that is not an
 * ordinary file inside ordinary directories is never read through or written
 * to. Content this version did not write is treated as absent, because every
 * value here can be learned again; the pairing list is a different store and
 * is never touched from here. */
public class KnownRouteStore(file: Path) {
    private val path = file.toAbsolutePath().normalize()
    private var current: SavedRoutes? = null
    private var usable = true

    /** False after the file could not be read or written safely. The values
     * still work for this run; they just will not survive it. */
    public val durable: Boolean @Synchronized get() = usable

    @Synchronized
    public fun load(): SavedRoutes {
        current?.let { return it }
        val loaded = try { readFile()?.let(SavedRoutes::decode) } catch (_: Exception) { usable = false; null }
        return (loaded ?: SavedRoutes()).also { current = it }
    }

    /** Apply a change and save it. The returned value is in effect for this
     * run even if saving failed; see [durable]. */
    @Synchronized
    public fun update(change: (SavedRoutes) -> SavedRoutes): SavedRoutes {
        val before = load()
        val after = change(before)
        if (after.sameAs(before)) return before
        current = after
        if (usable) usable = write(after.encode())
        return after
    }

    private fun write(bytes: ByteArray): Boolean {
        if (bytes.size > MAX_ROUTE_FILE_BYTES) return false
        var temporary: Path? = null
        return try {
            ordinaryPath(path)
            Files.createDirectories(checkNotNull(path.parent))
            ordinaryPath(path)
            temporary = Files.createTempFile(path.parent, ".known-routes-", ".tmp")
            Files.newOutputStream(temporary, StandardOpenOption.WRITE, StandardOpenOption.TRUNCATE_EXISTING,
                LinkOption.NOFOLLOW_LINKS).use { it.write(bytes) }
            ordinaryPath(path)
            Files.move(temporary, path, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING)
            temporary = null
            true
        } catch (_: Exception) { false }
        finally {
            temporary?.let { owned ->
                try { if (owned.parent == path.parent) { ordinaryPath(owned); Files.deleteIfExists(owned) } }
                catch (_: Exception) { /* Leave an uncertain temporary file alone. */ }
            }
        }
    }

    private fun readFile(): ByteArray? {
        ordinaryPath(path)
        val attributes = try { Files.readAttributes(path, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS) }
            catch (_: NoSuchFileException) { return null }
        check(attributes.isRegularFile)
        if (attributes.size() > MAX_ROUTE_FILE_BYTES) return ByteArray(0)
        return Files.newInputStream(path, LinkOption.NOFOLLOW_LINKS).use { input ->
            val output = ByteArrayOutputStream()
            val buffer = ByteArray(4096)
            while (true) {
                val count = input.read(buffer)
                if (count < 0) break
                if (output.size() + count > MAX_ROUTE_FILE_BYTES) return ByteArray(0)
                output.write(buffer, 0, count)
            }
            output.toByteArray()
        }
    }

    private fun ordinaryPath(target: Path) {
        var walked = checkNotNull(target.root)
        for (part in target) {
            walked = walked.resolve(part)
            val attributes = try { Files.readAttributes(walked, BasicFileAttributes::class.java, LinkOption.NOFOLLOW_LINKS) }
                catch (_: NoSuchFileException) { continue }
            check(!attributes.isSymbolicLink && !attributes.isOther &&
                (walked == target || attributes.isDirectory) && walked.toRealPath() == walked)
        }
    }

    public companion object {
        /** Routes are filed under the device and its exact certificate, so a
         * device that pairs again with a new certificate starts with none, and
         * the file names no device. Null for input that is not a pairing. */
        public fun peerKey(deviceId: String, fingerprint: String): String? {
            if (deviceId.isBlank() || deviceId.length > 128 || !fingerprint.matches(Regex("[0-9a-f]{64}"))) return null
            val digest = MessageDigest.getInstance("SHA-256")
                .digest(("route" + '\u0000' + deviceId + '\u0000' + fingerprint).toByteArray(Charsets.UTF_8))
            return digest.joinToString("") { (it.toInt() and 255).toString(16).padStart(2, '0') }
        }
    }
}
