package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** UI-dispatcher owner. App link replacement joins detach before native close;
 * old async replies cannot change the replacement indicator. */
public class AgentCaptureObserver(private val scope: CoroutineScope,
    private val capability: (ProjectLink) -> WorkbenchAgentCaptureStatus = ::agentCaptureStatus) {
    private val mutable = MutableStateFlow(AgentCaptureStatus())
    public val state: StateFlow<AgentCaptureStatus> = mutable.asStateFlow()
    private var generation = 0L
    private var admission = 0L
    private var connected = false
    private var connectionKey: Pair<SyncStatus?, SessionCarrier?> = null to null
    private var current: ProjectLink? = null
    private var work: Job? = null
    public fun attach(link: ProjectLink) {
        check(work == null && current == null)
        check(generation < Long.MAX_VALUE); val ticket = ++generation
        val initial = link.status()
        current = link; connected = initial.status in connectedStates; connectionKey = initial.status to initial.carrier; admission++
        mutable.value = AgentCaptureStatus()
        val api = try { capability(link) } catch (_: Exception) { return }
        val job = scope.launch(start = CoroutineStart.LAZY) {
            var sequence = 0uL
            try {
                while (isActive && current === link && generation == ticket) {
                    val guard = admission
                    val immediate = api.status()
                    val value = if (immediate.sequence > sequence) immediate else
                        withTimeoutOrNull(1000) { api.waitStatus(sequence) } ?: api.status()
                    if (current !== link || generation != ticket) break
                    if (guard != admission) { sequence = 0uL; continue }
                    val latest = api.status()
                    if (value.connectionEpoch != latest.connectionEpoch || value.sequence < latest.sequence) { sequence = 0uL; continue }
                    sequence = value.sequence
                    mutable.value = if (connected && value.connectionEpoch and 1uL == 1uL) value else AgentCaptureStatus()
                }
            } catch (cancel: CancellationException) { throw cancel }
            catch (_: Exception) { if (current === link && generation == ticket) mutable.value = AgentCaptureStatus() }
        }
        work = job
        job.invokeOnCompletion { if (work === job && generation == ticket) mutable.value = AgentCaptureStatus() }
        job.start()
    }
    /** Called synchronously from the actual link-status collector. */
    public fun connectionChanged(value: SessionStatus?) {
        val next = value?.status in connectedStates
        // Every connection status transition fences a query that started before
        // it, including equal image revisions across carrier replacement.
        val key = value?.status to value?.carrier
        if (!next || connected != next || key != connectionKey) { admission++; mutable.value = AgentCaptureStatus() }
        connectionKey = key
        connected = next
    }
    public suspend fun detach() {
        generation++; admission++; current = null; connected = false; connectionKey = null to null
        mutable.value = AgentCaptureStatus()
        val retiring = work
        withContext(NonCancellable) { retiring?.cancelAndJoin(); if (work === retiring) work = null }
    }
    private companion object { val connectedStates = setOf(SyncStatus.Syncing, SyncStatus.Synced) }
}
