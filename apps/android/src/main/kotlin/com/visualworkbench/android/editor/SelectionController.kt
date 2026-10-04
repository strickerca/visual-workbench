package com.visualworkbench.android.editor

import android.content.Context
import android.net.Uri
import android.provider.DocumentsContract
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collect
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

internal data class SelectionSaveState(val saving: Boolean = false, val message: String? = null,
    val completed: SelectionExportReceipt? = null)

/** Native selection calls share the existing project, and settle before its
 * close. CreateDocument grants ownership of the exact fresh destination URI. */
internal class SelectionController(
    context: Context,
    private val scope: CoroutineScope,
    val core: WorkbenchCore,
    refresh: suspend (SelectionReceipt?) -> Unit,
    capability: (WorkbenchProject) -> WorkbenchSelections = { it.selections() },
    private val destination: SelectionDestination = ProviderSelectionDestination(context),
) {
    private val cacheRoot = context.applicationContext.cacheDir
    val interaction = SelectionInteraction(core, scope, System::currentTimeMillis, refresh, capability)
    val state: StateFlow<SelectionUiState> = interaction.state
    private val mutableSave = MutableStateFlow(SelectionSaveState())
    val saved: StateFlow<SelectionSaveState> = mutableSave.asStateFlow()
    private var saveJob: Job? = null
    private var discardJob: Job? = null
    private var validPublication: AtomicBoolean? = null
    private var savingTicket: SelectionSaveTicket? = null

    fun bind(project: WorkbenchProject, document: DocumentSnapshot, attachment: Long, sequence: ULong, camera: Camera) {
        interaction.bind(project, document, attachment, sequence); interaction.viewport(camera); invalidate()
    }
    fun observe(change: ProjectChange) { interaction.observe(change.project, change.sequence); invalidate() }
    fun viewport(camera: Camera) { interaction.viewport(camera) }
    private fun invalidate() {
        savingTicket?.let { if (!interaction.matches(it)) validPublication?.set(false) }
        if (saved.value.completed?.let { it.binding != state.value.document?.binding || it.selection != state.value.selection?.selection } == true)
            mutableSave.value = saved.value.copy(completed = null)
    }
    fun ticket(kind: SelectionExportKind, crop: Boolean): SelectionSaveTicket? = interaction.saveTicket(kind, crop)
    /** Return false before claiming the URI; caller must discard that unused
     * CreateDocument URI. Once true, this job owns publication/failure cleanup. */
    fun save(ticket: SelectionSaveTicket, uri: Uri): Boolean {
        if (!scope.isActive || uri.scheme != "content" || saveJob?.isActive == true || !interaction.matches(ticket)) return false
        val valid = AtomicBoolean(true); validPublication = valid; savingTicket = ticket
        mutableSave.value = SelectionSaveState(true, "Preparing full-resolution PNG…")
        saveJob = scope.launch(start = CoroutineStart.UNDISPATCHED) {
            val work = RasterTransfers.Workspace(cacheRoot)
            var published = false
            val watcher = launch { state.collect { if (!interaction.matches(ticket)) valid.set(false) } }
            try {
                withContext(Dispatchers.IO) { work.create() }
                val output = work.file("output.png")
                val result = interaction.export(ticket, work.root.absolutePath, output.absolutePath)
                currentCoroutineContext().ensureActive()
                if (!interaction.matches(ticket)) throw SelectionFailure(SelectionFailureKind.Conflict)
                mutableSave.value = saved.value.copy(message = "Saving the completed PNG…")
                destination.write(uri, output, result.encodedBytes) { valid.get() }
                currentCoroutineContext().ensureActive()
                published = true
                // Cleanup still owns the workspace and possibly the destination URI.
                mutableSave.value = SelectionSaveState(saving = true, message = "PNG saved at its original resolution.", completed = result.takeIf { interaction.matches(ticket) })
            } catch (cancel: CancellationException) {
                mutableSave.value = SelectionSaveState(saving = true, message = "Save cancelled. The original and selection remain in the project."); throw cancel
            } catch (error: Exception) {
                mutableSave.value = SelectionSaveState(saving = true, message = if (error is SelectionFailure && error.kind == SelectionFailureKind.Conflict)
                    "The document or selection changed. Choose Export again." else "The PNG could not be saved. The original and selection remain in the project.")
            } finally {
                withContext(NonCancellable) {
                    watcher.cancelAndJoin()
                    try { if (work.isOpen) withContext(Dispatchers.IO) { work.close() } }
                    catch (_: Exception) {
                        mutableSave.value = mutableSave.value.copy(message = "Temporary export cleanup could not finish. The private selection workspace was preserved for recovery.")
                    }
                    finally {
                        try { if (!published && !destination.discard(uri)) mutableSave.value = SelectionSaveState(saving = true, message = "The export did not finish. A partial file may remain at the chosen destination; the original is preserved.") }
                        finally { valid.set(false); validPublication = null; savingTicket = null; mutableSave.value = saved.value.copy(saving = false) }
                    }
                }
            }
        }
        return true
    }
    fun discardUnused(uri: Uri) {
        if (uri.scheme != "content") return
        if (discardJob?.isActive == true) {
            mutableSave.value = SelectionSaveState(message = "An earlier destination is still being removed. An empty file may remain at the new location.")
            return
        }
        discardJob = scope.launch(start = CoroutineStart.UNDISPATCHED) { withContext(NonCancellable) {
            if (!destination.discard(uri)) mutableSave.value = SelectionSaveState(message = "The unused destination could not be removed. An empty file may remain there.")
        } }
    }
    fun cancel() { validPublication?.set(false); saveJob?.cancel(); interaction.cancelOperation() }
    fun background() { interaction.cancelGesture(); cancel() }
    suspend fun detach(): Unit = withContext(NonCancellable) {
        validPublication?.set(false); saveJob?.cancelAndJoin(); saveJob = null
        discardJob?.join(); discardJob = null
        interaction.detach(); mutableSave.value = SelectionSaveState()
    }
}

/** Injected in source tests; tests never read/write the real clipboard or any
 * external document provider. A provider is not an atomic filesystem rename:
 * failure removes only the fresh owned URI and reports unsuccessful cleanup. */
internal interface SelectionDestination {
    suspend fun write(uri: Uri, source: File, expected: ULong, current: () -> Boolean)
    suspend fun discard(uri: Uri): Boolean
}
internal class ProviderSelectionDestination(context: Context) : SelectionDestination {
    private val context = context.applicationContext
    private val io = ProviderIo(this.context.contentResolver)
    override suspend fun write(uri: Uri, source: File, expected: ULong, current: () -> Boolean): Unit = withContext(Dispatchers.IO) {
        require(uri.scheme == "content" && expected in 1uL..(64uL * 1024uL * 1024uL))
        require(source.isFile && source.length().toULong() == expected)
        if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
        io.open(uri, "wt").use { output ->
            io.nonBlocking(output)
            source.inputStream().use { input ->
                val buffer = ByteArray(64 * 1024); var copied = 0uL
                while (true) {
                    currentCoroutineContext().ensureActive()
                    if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
                    val count = input.read(buffer); if (count < 0) break
                    if (count == 0) continue
                    copied += count.toULong()
                    require(copied <= expected) { "Prepared PNG changed during publication" }
                    io.write(output.fileDescriptor, buffer, count)
                }
                require(copied == expected)
            }
        }
        if (!current()) throw SelectionFailure(SelectionFailureKind.Conflict)
    }
    override suspend fun discard(uri: Uri): Boolean = withContext(NonCancellable) {
        if (uri.scheme != "content") return@withContext false
        try { withTimeout(2_000) { io.perform {
            if (DocumentsContract.isDocumentUri(context, uri)) DocumentsContract.deleteDocument(context.contentResolver, uri)
            else context.contentResolver.delete(uri, null, null) > 0
        } } } catch (_: Exception) { false }
    }
}
