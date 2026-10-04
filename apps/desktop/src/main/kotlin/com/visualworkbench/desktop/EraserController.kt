package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.CoroutineScope

/** Uses the same project owner and ordered snapshot feed as SelectionController. */
internal class EraserController(val core: WorkbenchCore, scope: CoroutineScope,
    selections: SelectionController, refresh: suspend (EraserChange?) -> Unit) {
    val interaction = EraserInteraction(scope, core::mapPoints, core::newId, System::currentTimeMillis, refresh)
    val tools = EraserToolbox(interaction, selections.interaction, scope)
    val state = interaction.state
    private var project: WorkbenchProject? = null
    private var backend: EraserBackend? = null
    fun bind(value: WorkbenchProject, document: DocumentSnapshot, epoch: Long, sequence: ULong, camera: Camera) {
        check(project == null || project === value) { "Detach eraser work before replacing the project" }
        if (project == null) { project = value; backend = projectEraserBackend(value) { desktopEraseHits(it, core) } }
        interaction.bind(checkNotNull(backend), document, epoch, sequence); interaction.viewport(camera)
    }
    fun observe(change: ProjectChange) { interaction.observe(change.project, change.sequence) }
    fun viewport(camera: Camera) { interaction.viewport(camera) }
    suspend fun detach() { interaction.detach(); project = null; backend = null }
}
