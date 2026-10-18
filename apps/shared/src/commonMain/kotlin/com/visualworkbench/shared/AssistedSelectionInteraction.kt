package com.visualworkbench.shared

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** UI-dispatcher owner of one exact source and one encoded-image session. The
 * caller supplies its existing mutation mutex, metadata allocator and currentness
 * predicate (project identity, full source/revision, page and attachment epoch).
 * Call invalidate on ordered changes before refresh, and close before replacing
 * or closing the project. Retain this owner if close fails; never credit a failed
 * native retirement. No controller, selection math, runtime or model is created.
 *
 * afterMutation receives the ACTUAL selection receipt, or null after an attempted
 * edit without a receipt. Refresh even for null: cancellation may follow commit.
 * After refresh, pass the receipt's object ID to SelectionInteraction.useObject
 * for ordinary Add/Subtract paint and mask-math correction. */
public class AssistedSelectionInteraction(
    parent: CoroutineScope,
    public val source: AssistedSelectionSource,
    private val selections: WorkbenchSelections,
    private val mutations: Mutex,
    private val metadata: suspend () -> WorkflowMetadata,
    private val isCurrent: (AssistedSelectionSource) -> Boolean,
    private val afterMutation: suspend (SelectionReceipt?) -> Unit,
    private val resources: () -> SessionResources = ::sessionResources,
    private val pins: AssistedSelectionPins? = null,
    private val backend: AssistedSelectionBackend? = null,
    private val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL,
) {
    private val supervisor: Job = SupervisorJob(parent.coroutineContext[Job])
    private val scope: CoroutineScope = CoroutineScope(parent.coroutineContext + supervisor)
    private val requests: Channel<AssistedSelectionPromptRequest> = Channel(Channel.CONFLATED)
    private val closeGate: Mutex = Mutex()
    private val mutable: MutableStateFlow<AssistedSelectionState> = MutableStateFlow(
        if (pins == null || backend == null) AssistedSelectionState(AssistedSelectionPhase.Unavailable,
            issue = "Assisted selection is unavailable until a model, its licenses and a local runtime are admitted. Manual selection remains available.")
        else AssistedSelectionState(AssistedSelectionPhase.Ready))
    public val state: StateFlow<AssistedSelectionState> = mutable.asStateFlow()
    private var sequence: ULong = 0uL
    private var lastPrompt: AssistedSelectionPrompt? = null
    private var revoked: Boolean = false
    private var saving: Boolean = false
    private var runner: Job? = null
    private var operation: Job? = null
    private var runtime: AssistedSelectionRuntime? = null
    private var session: AssistedSelectionSession? = null
    private var runtimeCharge: SessionResourceCharge? = null
    private var coverageCharge: SessionResourceCharge? = null
    private var retirementFailure: Throwable? = null

    init {
        source.admitShape(memoryBudgetBytes)
        pins?.admitShape()
    }

    public val retaining: Boolean get() = runtime != null || session != null || runtimeCharge != null ||
        coverageCharge != null || runner?.isCompleted == false || operation?.isCompleted == false

    /** One in-flight prompt and one latest queued prompt. Obsolete results are
     * discarded, and prompt changes never re-encode the image. */
    public fun prompt(value: AssistedSelectionPrompt): Boolean {
        if (!current() || !state.value.canPrompt || !supervisor.isActive) return false
        val owned = value.owned(source)
        if (owned == lastPrompt) return true
        if (sequence == ULong.MAX_VALUE) throw SelectionFailure(SelectionFailureKind.Limit)
        sequence++; lastPrompt = owned
        mutable.value = state.value.copy(proposal = null, issue = null, selectionFailure = null,
            phase = if (session == null) AssistedSelectionPhase.Encoding else AssistedSelectionPhase.Proposing)
        if (!requests.trySend(AssistedSelectionPromptRequest(source, sequence, owned)).isSuccess) return false
        if (runner == null) start()
        return true
    }

    private fun current(): Boolean {
        if (revoked) return false
        if (isCurrent(source)) return true
        invalidate(); return false
    }

    private fun start(): Unit {
        val adapter = backend ?: return
        val admitted = pins ?: return
        var entered = false
        // Publish before executing any injected backend callback. A reentrant
        // invalidate/close must observe the actual accepted producer job.
        val work = scope.launch(start = CoroutineStart.LAZY) {
            entered = true
            try {
                currentCoroutineContext().ensureActive()
                if (!current()) return@launch
                runtime = adapter.admit(admitted)
                currentCoroutineContext().ensureActive()
                val actual = checkNotNull(runtime)
                val workingSet = actual.workingSetBytes
                if (actual.admittedPins != admitted || AssistedSelectionExecution.Cpu !in actual.executions || workingSet == 0uL)
                    throw SelectionFailure(SelectionFailureKind.Unsupported)
                // Fallback is derived from the real admitted capability set.
                var execution = listOf(AssistedSelectionExecution.Npu, AssistedSelectionExecution.Gpu,
                    AssistedSelectionExecution.Cpu).firstOrNull { it in actual.executions }
                    ?: throw SelectionFailure(SelectionFailureKind.Unsupported)
                val pixels = source.document.width.toULong() * source.document.height
                val importBytes = pixels * 20uL + 16uL * 1024uL * 1024uL + PROMPT_RESERVE
                if (workingSet > memoryBudgetBytes || importBytes > memoryBudgetBytes - workingSet)
                    throw SelectionFailure(SelectionFailureKind.Memory,
                        if (workingSet > ULong.MAX_VALUE - importBytes) ULong.MAX_VALUE else workingSet + importBytes,
                        memoryBudgetBytes)
                val pool = resources()
                coverageCharge = pool.reserveForeign(pixels * 4uL + PROMPT_RESERVE)
                runtimeCharge = pool.reserveForeign(workingSet)
                currentCoroutineContext().ensureActive()
                if (!current()) return@launch
                session = try { actual.encode(source, execution) }
                catch (error: SelectionFailure) {
                    if (error.kind != SelectionFailureKind.Unsupported || execution == AssistedSelectionExecution.Cpu) throw error
                    // encode's failure contract has already joined/released its
                    // partial owner. Only this inspected CPU capability may be
                    // attempted once; storage/cancellation/inference never retry.
                    currentCoroutineContext().ensureActive()
                    if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
                    execution = AssistedSelectionExecution.Cpu
                    actual.encode(source, execution)
                }
                currentCoroutineContext().ensureActive()
                val encoded = checkNotNull(session)
                if (encoded.source != source || encoded.execution != execution)
                    throw SelectionFailure(SelectionFailureKind.Corrupt)
                if (!current()) return@launch
                mutable.value = state.value.copy(execution = execution)
                for (request in requests) {
                    if (!current()) break
                    if (request.sequence != sequence) continue
                    mutable.value = state.value.copy(phase = AssistedSelectionPhase.Proposing, proposal = null)
                    val mask = encoded.propose(request)
                    currentCoroutineContext().ensureActive()
                    if (!current()) break
                    if (request.sequence != sequence) continue
                    if (mask.source != source || mask.sequence != request.sequence ||
                        mask.width != source.document.width || mask.height != source.document.height)
                        throw SelectionFailure(SelectionFailureKind.Corrupt)
                    // The byte count and copy budget are checked before adoption.
                    admitSelectionCoverage(mask.width, mask.height, mask.coverage.size, memoryBudgetBytes)
                    val proposal = AssistedSelectionProposal(source, request.sequence, mask.coverage.copyOf())
                    mutable.value = state.value.copy(phase = AssistedSelectionPhase.Proposal, proposal = proposal)
                }
            } catch (cancel: CancellationException) {
                // A backend may cancel just this operation while its parent
                // stays active. Retire this actor permanently; accepting more
                // requests would strand them behind a completed runner.
                requests.close()
                if (!saving) revoked = true
                mutable.value = state.value.copy(proposal = null,
                    phase = if (saving) AssistedSelectionPhase.Saving else
                        if (state.value.phase == AssistedSelectionPhase.Stale) AssistedSelectionPhase.Stale else AssistedSelectionPhase.Closed,
                    issue = state.value.issue ?: if (saving) null else "Proposal cancelled. Manual selection remains available.")
                // Accept deliberately cancels its prompt actor before import;
                // it retains the source permit and Saving phase until settled.
                throw cancel
            }
            catch (error: Exception) {
                revoked = true
                mutable.value = state.value.copy(phase = AssistedSelectionPhase.Failed, proposal = null,
                    issue = "The admitted local selection backend could not produce a valid proposal. Manual selection remains available.",
                    selectionFailure = error as? SelectionFailure)
            } finally {
                withContext(NonCancellable) {
                    mutable.value = state.value.copy(proposal = null)
                    // A failed retirement can still own producer output. Keep
                    // both reservations until its real handle closes.
                    try { retireBackend(); if (!saving) releaseCoverage() }
                    catch (error: Throwable) { retainRetirementFailure(error) }
                }
            }
        }
        runner = work
        work.invokeOnCompletion {
            // A pre-cancelled LAZY coroutine never enters its try/finally.
            // No backend has been admitted, but revoke its queued UI request.
            if (!entered) {
                revoked = true; requests.close()
                mutable.value = state.value.copy(phase = AssistedSelectionPhase.Closed, proposal = null)
            }
        }
        work.start()
    }

    /** Only this explicit action writes. It consumes the exact adopted proposal
     * once, with caller-supplied ordinary selection target/version and metadata.
     * No retry or new transaction is generated after cancellation or failure. */
    public fun accept(proposal: AssistedSelectionProposal, target: SelectionTarget,
        combine: SelectionCombine = SelectionCombine.Add): Boolean {
        if (!current() || !state.value.canAccept || state.value.proposal !== proposal || saving ||
            !supervisor.isActive || (target is SelectionTarget.New && combine != SelectionCombine.Add)) return false
        saving = true
        mutable.value = state.value.copy(phase = AssistedSelectionPhase.Saving, proposal = null, issue = null)
        var entered = false
        val work = scope.launch(start = CoroutineStart.LAZY) {
            entered = true
            var receipt: SelectionReceipt? = null
            var attempted = false
            try {
                // No model/native prompt worker may overlap the canonical edit.
                runner?.cancelAndJoin()
                retirementFailure?.let { throw it }
                mutations.withLock {
                    currentCoroutineContext().ensureActive()
                    if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
                    val identity = metadata()
                    currentCoroutineContext().ensureActive()
                    if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
                    val edit = SelectionEdit(source.binding, identity.transactionId, identity.deviceId,
                        identity.firstLamport, identity.createdAtMs, target,
                        SelectionOperation.ImportCoverage(source.document.width, source.document.height,
                            proposal.ownedCoverage, combine), memoryBudgetBytes)
                    attempted = true
                    receipt = selections.edit(edit)
                    // Record the real receipt before a suspending root refresh.
                    mutable.value = state.value.copy(receipt = receipt)
                }
            } catch (cancel: CancellationException) {
                mutable.value = state.value.copy(issue = "Cancelled; checking whether the selection was already saved.")
                throw cancel
            } catch (error: Exception) {
                mutable.value = state.value.copy(issue = "Selection import did not return a receipt. Refresh before making another edit.",
                    selectionFailure = error as? SelectionFailure)
            } finally {
                withContext(NonCancellable) {
                    // Join also when this Accept owner was cancelled while waiting
                    // for a non-cooperative encoder/prompt to settle.
                    runner?.cancelAndJoin()
                    try { if (attempted) afterMutation(receipt) }
                    catch (error: Exception) {
                        mutable.value = state.value.copy(issue = "Selection refresh did not finish; refresh the editor before continuing.",
                            selectionFailure = state.value.selectionFailure ?: error as? SelectionFailure)
                    } finally {
                        saving = false; revoked = true; requests.close()
                        try { if (backendRetired()) releaseCoverage() }
                        catch (error: Throwable) { retainRetirementFailure(error) }
                        mutable.value = state.value.copy(phase = if (receipt != null && retirementFailure == null) AssistedSelectionPhase.Saved else AssistedSelectionPhase.Failed,
                            proposal = null, receipt = receipt)
                    }
                }
            }
        }
        operation = work
        work.invokeOnCompletion {
            if (!entered) {
                saving = false; revoked = true; requests.close(); runner?.cancel()
                mutable.value = state.value.copy(phase = AssistedSelectionPhase.Closed, proposal = null)
                try { if (runner?.isCompleted != false && backendRetired()) releaseCoverage() }
                catch (error: Throwable) { retainRetirementFailure(error) }
            }
        }
        work.start()
        return true
    }

    /** Revoke immediately on source/revision/page/epoch changes. Settlement is
     * still owned here until close returns successfully. */
    public fun invalidate(): Unit {
        revoked = true
        // Closing the channel or cancelling a child can run retirement inline.
        // Publish first so its retained failure is never overwritten afterward.
        mutable.value = state.value.copy(phase = if (retirementFailure != null || state.value.phase == AssistedSelectionPhase.Failed)
            AssistedSelectionPhase.Failed else AssistedSelectionPhase.Stale, proposal = null)
        requests.close(); runner?.cancel(); operation?.cancel()
    }

    public fun cancel(): Unit {
        invalidate()
        if (retirementFailure != null || state.value.phase == AssistedSelectionPhase.Failed) return
        mutable.value = state.value.copy(phase = if (saving) AssistedSelectionPhase.Saving else AssistedSelectionPhase.Closed,
            issue = if (saving) "Cancelling; checking whether the selection was already saved." else "Proposal cancelled. Manual selection remains available.")
    }

    /** Cancellation of the caller cannot abandon real backend or native owners.
     * A failed close is reported and retains the exact failing handle/charge. */
    public suspend fun close(): Unit = withContext(NonCancellable) {
        cancel()
        operation?.join(); runner?.join()
        closeGate.withLock {
            try {
                retireBackend(); releaseCoverage(); retirementFailure = null
                supervisor.cancel()
                mutable.value = state.value.copy(phase = AssistedSelectionPhase.Closed, proposal = null)
            } catch (error: Throwable) {
                retainRetirementFailure(error)
                throw error
            }
        }
    }

    private suspend fun retireBackend(): Unit {
        session?.let { it.close(); session = null }
        runtime?.let { it.close(); runtime = null }
        runtimeCharge?.let { it.close(); runtimeCharge = null }
    }
    private fun backendRetired(): Boolean = session == null && runtime == null && runtimeCharge == null
    private fun releaseCoverage(): Unit {
        coverageCharge?.let { it.close(); coverageCharge = null }
    }
    private fun retainRetirementFailure(error: Throwable): Unit {
        retirementFailure = error
        mutable.value = state.value.copy(phase = AssistedSelectionPhase.Failed,
            issue = "Selection backend retirement is retained; close must finish before replacing the project.")
    }
    private companion object { private const val PROMPT_RESERVE: ULong = 16_384uL }
}
