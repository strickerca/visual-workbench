package com.visualworkbench.android.session

import android.content.Context
import com.visualworkbench.shared.LocalSessionAddress
import com.visualworkbench.shared.RouteDiscovery
import com.visualworkbench.shared.WorkbenchSessions
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Announces and looks for project listeners with the platform's own service
 * discovery while the app is open. Everything found is an untrusted address
 * hint; the paired computer is recognised only by its pinned certificate.
 * Main thread only, like [AndroidDiscovery]. */
internal class AndroidRouteDiscovery(private val context: Context) : RouteDiscovery {
    private var current: AndroidDiscovery? = null
    /** The addresses being announced; empty while only looking; null when stopped. */
    private var announced: List<String>? = null

    private suspend fun ensure(service: WorkbenchSessions, endpoints: List<String>) {
        if (announced == endpoints) return
        stop()
        // A failed start is not retried until what to announce changes or
        // discovery is stopped; remembered routes still work without it.
        announced = endpoints
        val created = AndroidDiscovery(context)
        try { created.start(service, endpoints); current = created }
        catch (cancel: CancellationException) { withContext(NonCancellable) { runCatching { created.close() } }; announced = null; throw cancel }
        catch (_: Exception) { withContext(NonCancellable) { runCatching { created.close() } } }
    }

    override suspend fun advertise(service: WorkbenchSessions, endpoints: List<String>) = ensure(service, endpoints.take(8))

    override suspend fun found(service: WorkbenchSessions, local: List<LocalSessionAddress>): List<String> {
        if (announced == null) ensure(service, emptyList())
        return current?.peers.orEmpty().flatMap { it.endpoints }
    }

    override suspend fun stop() {
        val previous = current
        current = null; announced = null
        if (previous != null) withContext(NonCancellable) { runCatching { previous.close() } }
    }
}
