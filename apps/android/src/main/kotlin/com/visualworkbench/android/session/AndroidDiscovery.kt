package com.visualworkbench.android.session

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** NSD records are untrusted bounded hints. Native validates TXT and numeric
 * endpoints; only a later pinned TLS handshake identifies a paired computer. */
internal class AndroidDiscovery(context: Context) {
    private val manager = context.getSystemService(NsdManager::class.java)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val requests = Channel<NsdServiceInfo>(64)
    private sealed interface Event {
        data class Found(val info: NsdServiceInfo) : Event
        data class Lost(val name: String) : Event
        data object Unavailable : Event
    }
    private val events = Channel<Event>(64)
    private val known = LinkedHashMap<String, NsdServiceInfo>()
    private val ids = HashMap<String, String>()
    private var catalog: SessionDiscovery? = null
    private var listener: NsdManager.DiscoveryListener? = null
    private var registration: NsdManager.RegistrationListener? = null
    @Volatile private var closed = false
    var peers by mutableStateOf<List<DiscoveredPeer>>(emptyList()); private set
    var problem by mutableStateOf<String?>(null); private set

    suspend fun start(service: WorkbenchSessions, boundEndpoints: List<String> = emptyList()) {
        check(catalog == null && !closed)
        val created = if (boundEndpoints.isEmpty()) service.browseDiscovery() else service.createDiscovery(boundEndpoints)
        if (closed) { withContext(NonCancellable) { created.close() }; return }
        catalog = created
        created.advertisement?.let { advertisement ->
            val record = NsdServiceInfo().apply {
                serviceName = advertisement.instance; serviceType = advertisement.serviceType; port = advertisement.port.toInt()
                advertisement.properties.forEach { setAttribute(it.key, it.value) }
            }
            val callback = object : NsdManager.RegistrationListener {
                override fun onServiceRegistered(info: NsdServiceInfo) {
                    if (closed) runCatching { manager.unregisterService(this) }
                }
                override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
                override fun onRegistrationFailed(info: NsdServiceInfo, code: Int) { events.trySend(Event.Unavailable) }
                override fun onUnregistrationFailed(info: NsdServiceInfo, code: Int) = Unit
            }
            registration = callback
            manager.registerService(record, NsdManager.PROTOCOL_DNS_SD, callback)
        }
        val callback = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(type: String) {
                if (closed) runCatching { manager.stopServiceDiscovery(this) }
            }
            override fun onDiscoveryStopped(type: String) = Unit
            override fun onStartDiscoveryFailed(type: String, code: Int) { events.trySend(Event.Unavailable) }
            override fun onStopDiscoveryFailed(type: String, code: Int) = Unit
            override fun onServiceFound(info: NsdServiceInfo) { events.trySend(Event.Found(info)) }
            override fun onServiceLost(info: NsdServiceInfo) { if (info.serviceName.length <= 128) events.trySend(Event.Lost(info.serviceName)) }
        }
        listener = callback
        manager.discoverServices("_vworkbench._udp.", NsdManager.PROTOCOL_DNS_SD, callback)
        scope.launch {
            for (event in events) {
                if (closed) break
                when (event) {
                    is Event.Found -> {
                        val info = event.info
                        if (info.serviceType.trimEnd('.') == "_vworkbench._udp" && info.serviceName.length <= 128 &&
                            (known.size < 64 || known.containsKey(info.serviceName))) {
                            known[info.serviceName] = info; requests.trySend(info)
                        }
                    }
                    is Event.Lost -> {
                        known.remove(event.name)
                        ids.remove(event.name)?.let { id -> runCatching { created.forget(id) } }
                        runCatching { peers = created.snapshot() }
                    }
                    Event.Unavailable -> problem = "Local discovery is unavailable. Enter the computer's address or scan its QR."
                }
            }
        }
        scope.launch {
            try {
                for (record in requests) {
                    try {
                    val resolved = withTimeout(5000) { resolve(record) }
                    if (closed || !known.containsKey(record.serviceName)) continue
                    val properties = resolved.attributes.entries.take(3).map { (key, bytes) ->
                        require(key.length <= 16 && bytes.size <= 64)
                        require(bytes.all { it.toInt() in 32..126 })
                        DiscoveryProperty(key, bytes.toString(Charsets.US_ASCII))
                    }
                    require(resolved.attributes.size == 2)
                    @Suppress("DEPRECATION")
                    val host = resolved.host?.hostAddress ?: continue
                    val address = if (':' in host) "[$host]:${resolved.port}" else "$host:${resolved.port}"
                    created.observe(resolved.serviceType.trimEnd('.'), properties, listOf(address))
                    properties.firstOrNull { it.key == "id" }?.value?.let { ids[record.serviceName] = it }
                    peers = created.snapshot()
                    } catch (cancel: CancellationException) { throw cancel }
                    catch (_: Exception) {
                        known.remove(record.serviceName)
                        ids.remove(record.serviceName)?.let { id -> runCatching { created.forget(id) } }
                        // One malformed/unavailable service cannot stop other hints.
                    }
                }
            } catch (cancel: CancellationException) { if (cancel is TimeoutCancellationException) problem = "Discovery resolution timed out. Retry discovery or enter the computer's address." else throw cancel }
            catch (_: Exception) { problem = "A discovery record could not be safely resolved. Retry discovery or use the computer's displayed address." }
        }
        scope.launch {
            while (isActive) {
                delay(20_000)
                if (closed) break
                known.values.toList().forEach { requests.trySend(it) }
                try { peers = created.snapshot() } catch (_: Exception) { break }
            }
        }
    }

    @Suppress("DEPRECATION")
    private suspend fun resolve(info: NsdServiceInfo): NsdServiceInfo = suspendCancellableCoroutine { continuation ->
        manager.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(service: NsdServiceInfo, error: Int) { if (continuation.isActive) continuation.resumeWithException(IllegalStateException("Discovery resolution failed")) }
            override fun onServiceResolved(service: NsdServiceInfo) { if (continuation.isActive) continuation.resume(service) }
        })
    }
    suspend fun close() {
        closed = true
        listener?.let { runCatching { manager.stopServiceDiscovery(it) } }; listener = null
        registration?.let { runCatching { manager.unregisterService(it) } }; registration = null
        events.close(); requests.close(); scope.coroutineContext[Job]?.cancelAndJoin()
        val previous = catalog; catalog = null; peers = emptyList(); known.clear(); ids.clear()
        if (previous != null) withContext(NonCancellable) { previous.close() }
    }
}
