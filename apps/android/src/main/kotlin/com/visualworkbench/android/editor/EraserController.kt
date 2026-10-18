package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.CoroutineScope

internal class EraserController(val core: WorkbenchCore, scope: CoroutineScope,
    selections: SelectionInteraction, private val hit: suspend (EraserHitQuery) -> List<String> = ::androidEraseHits,
    refresh: suspend (EraserChange?) -> Unit) {
    val interaction = EraserInteraction(scope, core::mapPoints, core::newId, System::currentTimeMillis, refresh)
    val tools = EraserToolbox(interaction, selections, scope)
    val state = interaction.state
    private var project: WorkbenchProject? = null
    private var backend: EraserBackend? = null
    fun bind(value: WorkbenchProject, document: DocumentSnapshot, epoch: Long, sequence: ULong, camera: Camera) {
        check(project == null || project === value) { "Detach eraser work before replacing the project" }
        if (project == null) { project = value; backend = projectEraserBackend(value, hit) }
        interaction.bind(checkNotNull(backend), document, epoch, sequence); interaction.viewport(camera)
    }
    fun observe(change: ProjectChange) { interaction.observe(change.project, change.sequence) }
    suspend fun detach() { interaction.detach(); project = null; backend = null }
}
