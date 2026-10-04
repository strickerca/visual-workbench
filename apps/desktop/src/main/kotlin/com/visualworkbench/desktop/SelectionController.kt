package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collect
import java.nio.file.Path
import java.util.concurrent.atomic.AtomicBoolean

internal data class SelectionSaveState(val saving: Boolean = false, val message: String? = null,
    val completed: SelectionExportReceipt? = null)

/** Shares the editor's UI scope and source epoch, but settles all native work
 * before the editor closes its project. No live clipboard or drag side effects. */
internal class SelectionController(
    private val scope: CoroutineScope,
    val core: WorkbenchCore,
    workRoot: Path,
    refresh: suspend (SelectionReceipt?) -> Unit,
    capability: (WorkbenchProject) -> WorkbenchSelections = { it.selections() },
    private val publication: suspend (Path, Path, ULong, () -> Boolean) -> Unit = ::publishExport,
) {
    val interaction = SelectionInteraction(core, scope, System::currentTimeMillis, refresh, capability)
    val state: StateFlow<SelectionUiState> = interaction.state
    private val files = TransferFiles(workRoot)
    private val mutableSave = MutableStateFlow(SelectionSaveState())
    val saved: StateFlow<SelectionSaveState> = mutableSave.asStateFlow()
    private var saveJob: Job? = null
    private var publicationCurrent: AtomicBoolean? = null
    private var savingTicket: SelectionSaveTicket? = null

    fun bind(project: WorkbenchProject, document: DocumentSnapshot, attachment: Long, sequence: ULong, camera: Camera) {
        interaction.bind(project, document, attachment, sequence); interaction.viewport(camera); invalidateSave()
    }
    fun observe(change: ProjectChange) { interaction.observe(change.project, change.sequence); invalidateSave() }
    fun viewport(camera: Camera) { interaction.viewport(camera) }
    fun cancelGesture() { interaction.cancelGesture() }
    fun ticket(kind: SelectionExportKind, crop: Boolean): SelectionSaveTicket? = interaction.saveTicket(kind, crop)
    private fun invalidateSave() {
        savingTicket?.let { if (!interaction.matches(it)) publicationCurrent?.set(false) }
        if (mutableSave.value.completed?.let { it.binding != state.value.document?.binding || it.selection != state.value.selection?.selection } == true)
            mutableSave.value = mutableSave.value.copy(completed = null)
    }
    /** The request is captured before opening FileDialog. A changed document
     * refuses publication rather than silently exporting the newer revision. */
    fun save(ticket: SelectionSaveTicket, destination: Path): Boolean {
        if (!scope.isActive || saveJob?.isActive == true || !interaction.matches(ticket)) return false
        if (!destination.isAbsolute || !destination.fileName.toString().endsWith(".png", ignoreCase = true)) {
            mutableSave.value = SelectionSaveState(message = "Choose a new absolute .png file path."); return false
        }
        val valid = AtomicBoolean(true); publicationCurrent = valid; savingTicket = ticket
        mutableSave.value = SelectionSaveState(saving = true, message = "Preparing full-resolution PNG…")
        saveJob = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            var workspace: TransferWorkspace? = null
            // StateFlow is thread-safe; the IO publication check only reads this
            // atomic permission, never mutable UI-confined controller fields.
            val watcher = launch {
                state.collect { if (!interaction.matches(ticket)) valid.set(false) }
            }
            try {
                workspace = files.create()
                val output = workspace.directory.resolve("selection.png")
                val result = interaction.export(ticket, workspace.directory.toString(), output.toString())
                currentCoroutineContext().ensureActive()
                if (!interaction.matches(ticket)) throw SelectionFailure(SelectionFailureKind.Conflict)
                mutableSave.value = mutableSave.value.copy(message = "Saving the completed PNG…")
                publication(output, destination, result.encodedBytes) { valid.get() }
                // Keep ownership visible until the non-cancellable cleanup below settles.
                mutableSave.value = SelectionSaveState(saving = true, message = "PNG saved at its original resolution.",
                    completed = result.takeIf { interaction.matches(ticket) })
            } catch (cancel: CancellationException) {
                mutableSave.value = SelectionSaveState(saving = true, message = "Save cancelled. The original and selection remain in the project."); throw cancel
            } catch (error: Exception) {
                mutableSave.value = SelectionSaveState(saving = true, message = when (error) {
                    is SelectionFailure -> if (error.kind == SelectionFailureKind.Conflict) "The document or selection changed. Choose Export again." else "PNG export could not finish (${error.kind.name}); the original is preserved."
                    else -> error.message?.take(240) ?: "PNG export could not finish; the original is preserved."
                })
            } finally {
                withContext(NonCancellable) {
                    watcher.cancelAndJoin()
                    try { workspace?.close() }
                    catch (_: Exception) {
                        mutableSave.value = mutableSave.value.copy(message = "Temporary export cleanup could not finish. The private selection workspace was preserved for recovery.")
                    }
                    finally { valid.set(false); publicationCurrent = null; savingTicket = null; mutableSave.value = mutableSave.value.copy(saving = false) }
                }
            }
        }
        return true
    }
    fun cancel() { publicationCurrent?.set(false); saveJob?.cancel(); interaction.cancelOperation() }
    suspend fun detach(): Unit = withContext(NonCancellable) {
        publicationCurrent?.set(false); saveJob?.cancelAndJoin(); saveJob = null
        interaction.detach(); mutableSave.value = SelectionSaveState()
    }
}
