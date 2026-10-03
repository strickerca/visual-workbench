package com.visualworkbench.desktop

import com.visualworkbench.bindings.host.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.sync.Semaphore
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

internal enum class AssistDeviceState { Available, Offline, Unauthorized, Unavailable }
internal data class AssistDevice(val serial: String, val state: AssistDeviceState) {
    override fun toString() = "AssistDevice(serial=redacted,state=$state)"
}
internal data class ReverseSelection(val adbPath: String, val serial: String, val phonePort: UShort, val hostPort: UShort) {
    override fun toString() = "ReverseSelection(tool=redacted,device=redacted,phonePort=$phonePort,hostPort=$hostPort)"
}
internal enum class ReverseState { Checking, ServerUnavailable, UnsupportedServer, MissingDevice, Offline, Unauthorized, MappingCreated, ExistingMapping, ContestedMapping, Refused, TimedOut, InvalidReply, Stopped }
internal data class ReverseSnapshot(val state: ReverseState, val created: ULong, val cycleMillis: UInt, val mappingMayRemain: Boolean, val phonePort: UShort, val hostPort: UShort)
internal enum class MetricFamily { Ipv4, Ipv6 }
internal enum class MetricKind { Fix, Revert }
internal data class MetricDetails(val kind: MetricKind, val family: MetricFamily, val interfaceIndex: UInt,
    val oldAutomatic: Boolean, val oldMetric: UInt, val newAutomatic: Boolean, val newMetric: UInt)
internal data class MetricRecovery(val family: MetricFamily, val interfaceIndex: UInt, val originalAutomatic: Boolean, val originalMetric: UInt)
internal interface ReverseLease { fun snapshot(): ReverseSnapshot; suspend fun close() }
internal interface MetricLease { val details: MetricDetails; suspend fun apply(): MetricLease; fun close() }
internal interface ConnectionAssistancePort {
    suspend fun devices(path: String): List<AssistDevice>
    suspend fun watch(selection: ReverseSelection): ReverseLease
    suspend fun metric(interfaceIndex: UInt, family: MetricFamily): MetricLease
    suspend fun close()
}
internal class AssistanceFailure(message: String, val retryableMetric: Boolean = false) : Exception(message)

/** This producer scope outlives cancellation of a dialog request. It signals
 * native cancellation and settles the operation before disposing late handles.
 * No connection action is invoked by constructing this adapter. */
internal class NativeConnectionAssistance : ConnectionAssistancePort {
    private val producers = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val slots = Semaphore(2)
    private val closed = AtomicBoolean(false)
    private suspend fun <T> call(discard: suspend (T) -> Unit = {}, invoke: suspend (ConnectionRequest) -> T): T {
        if (closed.get() || !slots.tryAcquire()) throw AssistanceFailure("Connection assistance is busy or closed.", retryableMetric = true)
        try {
            val request = ConnectionRequest()
            val owned = AtomicReference<T?>(null)
            try {
                val producer = producers.async { invoke(request).also { owned.set(it) } }
                try {
                    val result = producer.await()
                    currentCoroutineContext().ensureActive()
                    owned.set(null)
                    return result
                } catch (cancel: CancellationException) {
                    request.cancel()
                    withContext(NonCancellable) {
                        try { producer.await() } catch (_: Exception) { }
                        owned.getAndSet(null)?.let { discard(it) }
                    }
                    throw cancel
                } catch (error: ConnectionAssistException) {
                    throw AssistanceFailure(assistanceMessage(error), error is ConnectionAssistException.ElevationDeclined
                        || error is ConnectionAssistException.Busy || error is ConnectionAssistException.Cancelled)
                }
            } finally { request.destroy() }
        } finally { slots.release() }
    }
    override suspend fun devices(path: String): List<AssistDevice> = call { token ->
        listAdbDevices(path, token).map { AssistDevice(it.serial, when (it.state) {
            AdbDeviceState.AVAILABLE -> AssistDeviceState.Available
            AdbDeviceState.OFFLINE -> AssistDeviceState.Offline
            AdbDeviceState.UNAUTHORIZED -> AssistDeviceState.Unauthorized
            AdbDeviceState.UNAVAILABLE -> AssistDeviceState.Unavailable
        }) }
    }
    override suspend fun watch(selection: ReverseSelection): ReverseLease = call(discard = { it.close() }) { token ->
        NativeReverseLease(startAdbReverse(AdbReverseConfig(selection.adbPath, selection.serial, selection.phonePort, selection.hostPort), token))
    }
    override suspend fun metric(interfaceIndex: UInt, family: MetricFamily): MetricLease = call(discard = { it.close() }) { token ->
        val native = prepareRouteMetricFix(interfaceIndex, family.native(), token)
        try { NativeMetricLease(native) } catch (error: Throwable) { native.destroy(); throw error }
    }
    private inner class NativeMetricLease(private val native: RouteMetricAction) : MetricLease {
        private val released = AtomicBoolean(false)
        override val details = native.details().let { MetricDetails(if (it.kind == MetricActionKind.FIX) MetricKind.Fix else MetricKind.Revert,
            if (it.family == RouteFamily.IPV4) MetricFamily.Ipv4 else MetricFamily.Ipv6, it.interfaceIndex,
            it.oldAutomatic, it.oldMetric, it.newAutomatic, it.newMetric) }
        override suspend fun apply(): MetricLease = call(discard = { it.close() }) { token ->
            val receipt = native.apply(token)
            try { NativeMetricLease(receipt.inverse) }
            catch (error: Throwable) { receipt.inverse.destroy(); throw error }
        }
        override fun close() { if (released.compareAndSet(false, true)) native.destroy() }
    }
    private class NativeReverseLease(private val native: AdbReverseWatch) : ReverseLease {
        private val released = AtomicBoolean(false)
        override fun snapshot(): ReverseSnapshot = native.snapshot().let { ReverseSnapshot(when (it.state) {
            AdbReverseState.CHECKING -> ReverseState.Checking
            AdbReverseState.SERVER_UNAVAILABLE -> ReverseState.ServerUnavailable
            AdbReverseState.UNSUPPORTED_SERVER -> ReverseState.UnsupportedServer
            AdbReverseState.MISSING_DEVICE -> ReverseState.MissingDevice
            AdbReverseState.OFFLINE -> ReverseState.Offline
            AdbReverseState.UNAUTHORIZED -> ReverseState.Unauthorized
            AdbReverseState.MAPPING_CREATED -> ReverseState.MappingCreated
            AdbReverseState.EXISTING_MAPPING -> ReverseState.ExistingMapping
            AdbReverseState.CONTESTED_MAPPING -> ReverseState.ContestedMapping
            AdbReverseState.REFUSED -> ReverseState.Refused
            AdbReverseState.TIMED_OUT -> ReverseState.TimedOut
            AdbReverseState.INVALID_REPLY -> ReverseState.InvalidReply
            AdbReverseState.STOPPED -> ReverseState.Stopped
        }, it.createdCount, it.cycleMillis, it.createdMappingMayRemain, it.phonePort, it.hostPort) }
        override suspend fun close() {
            if (released.compareAndSet(false, true)) withContext(NonCancellable) { try { native.shutdown() } finally { native.destroy() } }
        }
    }
    override suspend fun close() {
        if (closed.compareAndSet(false, true)) producers.coroutineContext[Job]?.cancelAndJoin()
    }
}
private fun MetricFamily.native() = if (this == MetricFamily.Ipv4) RouteFamily.IPV4 else RouteFamily.IPV6
private fun assistanceMessage(error: ConnectionAssistException): String = when (error) {
    is ConnectionAssistException.InvalidAdbTool -> "Choose a trusted, installed local adb.exe. It will not be downloaded or replaced."
    is ConnectionAssistException.UnsupportedAdb -> "This tool or existing ADB server uses an unsupported protocol. The server was left running unchanged."
    is ConnectionAssistException.AdbServerUnavailable -> "The local ADB server is unavailable. Start your selected ADB tool separately, then inspect devices again."
    is ConnectionAssistException.StaleRoute -> "Routes or metrics changed. Refresh the proposal before approving another change."
    is ConnectionAssistException.NoMetricFix -> "This interface has no safe metric fix to offer. It may already be lower priority, or it is the only default route."
    is ConnectionAssistException.ElevationDeclined -> "Administrator approval was declined."
    is ConnectionAssistException.Cancelled -> "The native request was cancelled before a metric setter ran."
    is ConnectionAssistException.HelperUnavailable -> "The installed connection helper is unavailable. No administrator action was started."
    is ConnectionAssistException.MetricOutcomeUnknown -> "The metric outcome is uncertain. Refresh the routing state before changing or reverting it."
    is ConnectionAssistException.MetricFailed -> "Windows did not confirm the metric change. Refresh the routing state."
    is ConnectionAssistException.Timeout -> "The connection assistance deadline expired. Retry explicitly."
    is ConnectionAssistException.Busy -> "Another connection assistance operation is still settling."
    else -> "Connection assistance could not complete this request. No successful connection or route change is claimed."
}

internal data class AssistanceState(val adbPath: String = "", val devices: List<AssistDevice> = emptyList(), val selectedSerial: String? = null,
    val busy: Boolean = false, val watching: Boolean = false, val reverse: ReverseSnapshot? = null,
    val metric: MetricDetails? = null, val reverts: List<MetricDetails> = emptyList(), val recoveries: List<MetricRecovery> = emptyList(), val message: String? = null)

/** App-owned, UI-thread controller. Closing the dialog leaves an opted-in watch
 * running; Stop and app shutdown cancel and await the owned native watcher.
 * Nothing here persists or prints tool paths, serials or routing snapshots. */
internal class ConnectionAssistanceController(private val scope: CoroutineScope, private val port: ConnectionAssistancePort) {
    private val mutable = MutableStateFlow(AssistanceState())
    val state: StateFlow<AssistanceState> = mutable
    private var work: Job? = null
    private var polling: Job? = null
    private var watch: ReverseLease? = null
    private var metric: MetricLease? = null
    private val reverts = linkedMapOf<Pair<UInt, MetricFamily>, MetricLease>()
    private val recoveries = linkedMapOf<Pair<UInt, MetricFamily>, MetricRecovery>()
    private var closed = false
    private var epoch = 0L
    fun selectTool(path: String) {
        if (closed || state.value.busy || watch != null) return
        if (path.length > 32_000 || path.contains('\u0000')) { message("Choose a local installed adb.exe."); return }
        mutable.value = state.value.copy(adbPath = path, devices = emptyList(), selectedSerial = null, message = null)
    }
    fun selectDevice(serial: String) {
        if (!closed && !state.value.busy && watch == null && state.value.devices.any { it.serial == serial })
            mutable.value = state.value.copy(selectedSerial = serial, message = null)
    }
    fun inspectDevices() {
        val path = state.value.adbPath
        if (path.isBlank() || watch != null) return
        launch { ticket ->
            val devices = port.devices(path)
            currentCoroutineContext().ensureActive()
            if (current(ticket)) mutable.value = state.value.copy(devices = devices.toList(), selectedSerial = null,
                message = if (devices.isEmpty()) "No devices are listed by the existing local ADB server." else "Select the device you want this app to use.")
        }
    }
    fun startReverse(phone: String, host: String) {
        val value = state.value
        if (watch != null || value.selectedSerial == null || value.devices.none { it.serial == value.selectedSerial }) return
        val phonePort = portNumber(phone); val hostPort = portNumber(host)
        if (phonePort == null || hostPort == null) { message("Use explicit TCP ports from 1024 through 65535."); return }
        val selection = ReverseSelection(value.adbPath, value.selectedSerial, phonePort, hostPort)
        launch { ticket ->
            var owned: ReverseLease? = port.watch(selection)
            try {
                currentCoroutineContext().ensureActive()
                if (current(ticket)) {
                    val active = checkNotNull(owned); val snapshot = active.snapshot()
                    watch = active; owned = null
                    mutable.value = state.value.copy(watching = true, reverse = snapshot,
                        message = "Watching the selected device. Closing this dialog keeps recovery enabled; Stop leaves reverse mappings in place.")
                    polling = scope.launch {
                        try { while (isActive && !closed && watch === active) {
                            mutable.value = state.value.copy(reverse = active.snapshot()); delay(250)
                        } } catch (cancel: CancellationException) { throw cancel }
                        catch (_: Exception) { if (!closed && watch === active) message("The ADB watch status is unavailable. Stop it before starting another.") }
                    }
                }
            } finally { withContext(NonCancellable) { owned?.close() } }
        }
    }
    fun stopReverse() {
        if (closed || state.value.busy) return
        launch { _ -> stopOwnedWatch() }
    }
    private suspend fun stopOwnedWatch() {
        polling?.cancelAndJoin(); polling = null
        val old = watch; watch = null
        if (old != null) withContext(NonCancellable) {
            try { old.close() } finally {
                if (!closed) mutable.value = state.value.copy(watching = false,
                    reverse = state.value.reverse?.copy(state = ReverseState.Stopped),
                    message = "Recovery stopped. Reverse mappings were left in place; this app did not remove any mapping.")
            }
        }
    }
    fun prepareMetric(index: UInt, family: MetricFamily) {
        if (index == 0u) return
        if (reverts.containsKey(index to family)) { message("A verified change on this interface already has a retained Revert action below. Use it before preparing another fix."); return }
        if ((index to family) !in recoveries && (reverts.keys + recoveries.keys).size >= 8) { message("Eight metric changes have retained recovery values. Resolve them before preparing another interface."); return }
        launch { ticket ->
            var owned: MetricLease? = port.metric(index, family)
            try {
                currentCoroutineContext().ensureActive()
                if (current(ticket)) {
                    metric?.close(); metric = owned; owned = null
                    mutable.value = state.value.copy(metric = metric?.details, message = "Review the captured values. Apply will ask Windows for administrator approval.")
                }
            } finally { owned?.close() }
        }
    }
    fun applyMetric() {
        if (closed || state.value.busy) return
        val action = metric ?: return
        metric = null
        mutable.value = state.value.copy(metric = null)
        applyOwnedMetric(action)
    }
    fun revertMetric(index: UInt, family: MetricFamily) {
        if (closed || state.value.busy) return
        if (reverts.keys.lastOrNull() != (index to family)) { message("Revert the most recent metric change first. Each receipt is bound to the complete route table."); return }
        val action = reverts[index to family] ?: return
        applyOwnedMetric(action)
    }
    private fun applyOwnedMetric(action: MetricLease) {
        launch { ticket ->
            var inverse: MetricLease? = null
            var retained = false
            val details = action.details
            val key = details.interfaceIndex to details.family
            try {
                inverse = action.apply()
                currentCoroutineContext().ensureActive()
                if (current(ticket)) {
                    val returned = checkNotNull(inverse)
                    if (returned.details.kind == MetricKind.Revert) {
                        reverts.put(key, returned)?.close(); inverse = null
                    } else if (reverts[key] === action) {
                        reverts.remove(key)
                    }
                    val recovery = recoveries[key]
                    if (details.kind == MetricKind.Revert && recovery != null && recovery.originalAutomatic == details.newAutomatic
                        && recovery.originalMetric == details.newMetric) recoveries.remove(key)
                    mutable.value = state.value.copy(reverts = reverts.values.map { it.details }, recoveries = recoveries.values.toList(),
                        message = "Windows metric change verified. Retained Revert actions are bound to their observed routes and refuse later changes.")
                }
            } catch (error: AssistanceFailure) {
                if (error.retryableMetric) {
                    // The native action was re-armed only after proving no
                    // setter ran. Keep the exact captured inverse for retry.
                    if (details.kind == MetricKind.Revert) retained = reverts[key] === action
                    else if (current(ticket)) { metric = action; retained = true }
                } else retainRecovery(key, details)
                throw error
            } catch (error: Exception) {
                // Cancellation/unknown outcome must never erase the last known
                // original values, even though the consumed action cannot run.
                retainRecovery(key, details)
                throw error
            } finally {
                inverse?.close()
                if (!retained) { if (reverts[key] === action) reverts.remove(key); action.close() }
                if (current(ticket)) mutable.value = state.value.copy(metric = metric?.details,
                    reverts = reverts.values.map { it.details }, recoveries = recoveries.values.toList())
            }
        }
    }
    private fun retainRecovery(key: Pair<UInt, MetricFamily>, details: MetricDetails) {
        recoveries.putIfAbsent(key, MetricRecovery(details.family, details.interfaceIndex,
            if (details.kind == MetricKind.Revert) details.newAutomatic else details.oldAutomatic,
            if (details.kind == MetricKind.Revert) details.newMetric else details.oldMetric))
    }
    fun resolveRecovery(index: UInt, family: MetricFamily) {
        if (closed || state.value.busy) return
        recoveries.remove(index to family)
        mutable.value = state.value.copy(recoveries = recoveries.values.toList(),
            message = "Saved recovery values dismissed by your action. No Windows metric was changed.")
    }
    fun cancelWork() { work?.cancel() }
    private fun launch(block: suspend (Long) -> Unit) {
        if (closed || state.value.busy) return
        val ticket = ++epoch
        mutable.value = state.value.copy(busy = true, message = null)
        work = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            try { block(ticket) }
            catch (cancel: CancellationException) { if (current(ticket)) message("Request cancelled. A metric setter already in flight may have completed; refresh its state before retrying."); throw cancel }
            catch (error: AssistanceFailure) { if (current(ticket)) message(error.message ?: "Connection assistance could not complete.") }
            catch (_: Exception) { if (current(ticket)) message("Connection assistance could not complete. Refresh its state before retrying.") }
            finally { if (current(ticket)) mutable.value = state.value.copy(busy = false) }
        }
    }
    private fun current(ticket: Long) = !closed && epoch == ticket
    private fun message(text: String) { if (!closed) mutable.value = state.value.copy(message = text) }
    suspend fun close() {
        if (closed) return
        closed = true; epoch++
        withContext(NonCancellable) {
            try { work?.cancelAndJoin(); stopOwnedWatch() }
            finally { try { metric?.close(); metric = null; reverts.values.forEach { it.close() }; reverts.clear(); recoveries.clear() } finally { port.close() } }
        }
        mutable.value = AssistanceState(message = "Connection assistance closed. Existing reverse mappings were preserved.")
    }
}
private fun portNumber(value: String): UShort? = value.takeIf { it.length in 1..5 && it.all(Char::isDigit) }
    ?.toIntOrNull()?.takeIf { it in 1024..65535 }?.toUShort()

internal fun reverseLabel(value: ReverseSnapshot): String = when (value.state) {
    ReverseState.Checking -> "Checking the selected device"
    ReverseState.ServerUnavailable -> "Existing local ADB server unavailable; it was not started or reset"
    ReverseState.UnsupportedServer -> "Existing ADB server protocol unsupported; it was left unchanged"
    ReverseState.MissingDevice -> "Selected device is missing"
    ReverseState.Offline -> "Selected device is offline"
    ReverseState.Unauthorized -> "Selected device has not authorized this PC"
    ReverseState.MappingCreated -> "Created the selected reverse mapping"
    ReverseState.ExistingMapping -> "Matching mapping already exists; its ownership is shared or unknown"
    ReverseState.ContestedMapping -> "Selected phone port belongs to another mapping; nothing was rebound"
    ReverseState.Refused -> "The selected mapping request was refused"
    ReverseState.TimedOut -> "ADB response exceeded the cycle deadline"
    ReverseState.InvalidReply -> "ADB returned an invalid or oversized response"
    ReverseState.Stopped -> "Recovery stopped; mappings were preserved"
}
