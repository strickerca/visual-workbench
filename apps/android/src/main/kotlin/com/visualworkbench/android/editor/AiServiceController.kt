package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

internal data class AiServiceState(
    val busy: Boolean = false,
    val configuration: AiConfiguration? = null,
    val readiness: AiReadiness? = null,
    val failure: AiFailureKind? = null,
    val closed: Boolean = false,
)

/** Application-scope owner, confined to its UI scope. Opening settings may read
 * readiness but never sends or initializes a missing budget ledger. Only the
 * separate explicit initialize() action may provision first-use private state. */
internal class AiServiceController(
    private val scope: CoroutineScope,
    private val privateDirectory: String,
    private val create: suspend (String, Boolean) -> WorkbenchAiService =
        { path, initialize -> createAiService(path, provisionFirstInstall = initialize) },
) {
    private val mutable = MutableStateFlow(AiServiceState())
    val state: StateFlow<AiServiceState> = mutable.asStateFlow()
    private var service: WorkbenchAiService? = null
    private var operation: Job? = null
    private var closing = false
    private val closed = CompletableDeferred<Result<Unit>>()

    fun activate() { load(false) }
    fun initialize() { load(true) }
    fun requireService(): WorkbenchAiService {
        if (closing) throw AiFailure(AiFailureKind.Closed)
        if (operation?.isCompleted == false) throw AiFailure(AiFailureKind.Busy)
        return service ?: throw AiFailure(AiFailureKind.Unsupported)
    }

    private fun load(initialize: Boolean) = launch {
        val existing = service
        val selected = existing ?: create(privateDirectory, initialize)
        var adopted = existing != null
        try {
            val configuration = selected.configuration()
            val readiness = selected.readiness()
            currentCoroutineContext().ensureActive()
            if (closing) throw CancellationException("AI settings closing")
            service = selected
            adopted = true
            mutable.value = AiServiceState(busy = true, configuration = configuration, readiness = readiness)
        } finally {
            if (!adopted) withContext(NonCancellable) { selected.close() }
        }
    }

    fun configure(json: String, expectedFingerprint: String) {
        // Bound before UTF-8 conversion. Configuration contains model/cost policy,
        // never a provider key. Native validates the schema and exact fingerprint.
        if (json.length > 65_536 || json.encodeToByteArray().size > 65_536) {
            fail(AiFailureKind.Limit); return
        }
        launch {
            val owner = service ?: throw AiFailure(AiFailureKind.Unsupported)
            val configuration = owner.configure(json, expectedFingerprint)
            val readiness = owner.readiness()
            currentCoroutineContext().ensureActive()
            if (closing) throw CancellationException("AI settings closing")
            mutable.value = mutable.value.copy(configuration = configuration, readiness = readiness)
        }
    }

    fun configureDailyBudget(expectedFingerprint: String, microusd: ULong) = launch {
        val owner = service ?: throw AiFailure(AiFailureKind.Unsupported)
        val configuration = owner.configureDailyBudget(expectedFingerprint, microusd)
        val readiness = owner.readiness()
        currentCoroutineContext().ensureActive()
        if (closing) throw CancellationException("AI settings closing")
        mutable.value = mutable.value.copy(configuration = configuration, readiness = readiness)
    }

    /** Windows settings only. Caller hands over this bounded byte array; it is
     * wiped on every branch, including admission refusal and never-started work.
     * Android uses the existing ProviderKeyDialog/AndroidProviderKeyStore. */
    fun saveWindowsKey(bytes: ByteArray) {
        if (bytes.size !in 1..2560 || bytes.any { (it.toInt() and 255) !in 33..126 }) {
            bytes.fill(0); fail(AiFailureKind.Credentials); return
        }
        if (closing || operation?.isCompleted == false) {
            bytes.fill(0); fail(if (closing) AiFailureKind.Closed else AiFailureKind.Busy); return
        }
        val job = launch {
            try {
                val owner = service ?: throw AiFailure(AiFailureKind.Unsupported)
                owner.saveWindowsKey(bytes)
                val readiness = owner.readiness()
                currentCoroutineContext().ensureActive()
                if (closing) throw CancellationException("AI settings closing")
                mutable.value = mutable.value.copy(readiness = readiness)
            } finally { bytes.fill(0) }
        }
        // launch may be refused/cancelled before its body gets ownership.
        if (job == null) bytes.fill(0) else job.invokeOnCompletion { bytes.fill(0) }
    }
    fun removeWindowsKey() = launch {
        val owner = service ?: throw AiFailure(AiFailureKind.Unsupported)
        owner.removeWindowsKey()
        val readiness = owner.readiness()
        currentCoroutineContext().ensureActive()
        if (closing) throw CancellationException("AI settings closing")
        mutable.value = mutable.value.copy(readiness = readiness)
    }
    fun cancelSettings() { operation?.cancel() }
    fun clearFailure() { mutable.value = mutable.value.copy(failure = null) }

    private fun fail(kind: AiFailureKind) {
        mutable.value = mutable.value.copy(failure = kind)
    }
    private fun launch(block: suspend () -> Unit): Job? {
        if (closing || operation?.isCompleted == false) {
            fail(if (closing) AiFailureKind.Closed else AiFailureKind.Busy)
            return null
        }
        mutable.value = mutable.value.copy(busy = true, failure = null)
        val work = scope.launch(start = CoroutineStart.LAZY) {
            try { block() }
            catch (cancelled: CancellationException) {
                if (!closing) fail(AiFailureKind.Cancelled)
                throw cancelled
            }
            catch (failure: AiFailure) { if (!closing) fail(failure.kind) }
            catch (_: Exception) { if (!closing) fail(AiFailureKind.Unsupported) }
        }
        operation = work
        // Keep the slot while a cancelled producer settles under NonCancellable.
        // Completion also covers a cancelled scope that never enters the body.
        work.invokeOnCompletion {
            if (operation === work) {
                operation = null
                if (!closing) mutable.value = mutable.value.copy(busy = false)
            }
        }
        work.start()
        return work
    }

    /** Project request and tile owners must settle before this application owner.
     * Idempotent; a cancelled caller still waits for actual service closure. */
    suspend fun close() {
        if (closing) { withContext(NonCancellable) { closed.await().getOrThrow() }; return }
        closing = true
        mutable.value = mutable.value.copy(closed = true, busy = true, configuration = null, readiness = null)
        withContext(NonCancellable) {
            var outcome: Result<Unit> = Result.success(Unit)
            try {
                operation?.cancelAndJoin()
                val owner = service
                service = null
                owner?.close()
            } catch (error: Throwable) {
                outcome = Result.failure(error)
                throw error
            } finally {
                mutable.value = mutable.value.copy(busy = false)
                closed.complete(outcome)
            }
        }
    }
}

internal fun aiMoney(microusd: ULong): String {
    val dollars = microusd / 1_000_000uL
    val fraction = (microusd % 1_000_000uL).toString().padStart(6, '0').trimEnd('0')
    return "$" + dollars + if (fraction.isEmpty()) ".00" else "." + fraction.padEnd(2, '0')
}
/** Exact decimal dollars with at most six fractional digits. No Float money. */
internal fun aiParseMoney(value: String): ULong? {
    if (value.isEmpty() || value.length > 27 || !Regex("[0-9]+(\\.[0-9]{1,6})?").matches(value)) return null
    val parts = value.split('.')
    val whole = parts[0].toULongOrNull() ?: return null
    val fraction = if (parts.size == 2) parts[1].padEnd(6, '0').toULong() else 0uL
    if (whole > (ULong.MAX_VALUE - fraction) / 1_000_000uL) return null
    return whole * 1_000_000uL + fraction
}
internal fun aiBudgetExceeded(readiness: AiReadiness, estimate: ULong): Boolean =
    estimate > readiness.dailySoftBudgetMicrousd ||
        readiness.spentTodayMicrousd > readiness.dailySoftBudgetMicrousd - estimate.coerceAtMost(readiness.dailySoftBudgetMicrousd)

internal fun aiFailureText(kind: AiFailureKind): String = when (kind) {
    AiFailureKind.Invalid -> "Check the input values and review the request again."
    AiFailureKind.Stale -> "The document, source, masks or settings changed. Prepare a fresh review before sending."
    AiFailureKind.Limit -> "This operation exceeds the supported size or complexity. Reduce its scope."
    AiFailureKind.Depth -> "Review and explicitly allow an 8-bit provider copy; the original stays unchanged."
    AiFailureKind.Color -> "The source color profile needs explicit supported handling before sending."
    AiFailureKind.Estimate -> "A current, supported token estimate is required before Send."
    AiFailureKind.SoftBudget -> "The estimated request exceeds the daily soft budget. Review the warning and explicitly acknowledge it."
    AiFailureKind.Unresolved -> "A previous attempt has an uncertain charge. No automatic retry or budget reset is allowed."
    AiFailureKind.AttemptConsumed -> "This request has already used its one send attempt. It will not be sent again."
    AiFailureKind.Credentials -> "Protected provider credentials are unavailable. Open API key settings."
    AiFailureKind.Provider -> "The provider request failed. Its charge may be uncertain; it will not be retried automatically."
    AiFailureKind.Proof -> "The outside-region proof failed. This response cannot be accepted."
    AiFailureKind.Storage -> "Private AI state is unavailable or damaged. Existing budget history is preserved."
    AiFailureKind.Unsupported -> "AI editing is unavailable in this platform binding or configuration."
    AiFailureKind.Cancelled -> "Operation cancelled. A request already sent may still be charged."
    AiFailureKind.Closed -> "This AI session has closed."
    AiFailureKind.Busy -> "Finish or cancel the current AI operation first."
}
