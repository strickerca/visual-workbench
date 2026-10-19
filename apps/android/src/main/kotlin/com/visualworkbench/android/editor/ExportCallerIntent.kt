package com.visualworkbench.android.editor

import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.ExportOptions
import com.visualworkbench.shared.ImageOutputExportGeometry
import com.visualworkbench.shared.ProjectInfo
import com.visualworkbench.shared.WorkflowBinding
import com.visualworkbench.shared.WorkbenchProject

/** The source D request survives the picker separately from one resolved O request.
 * A retry must project the original request again, never the previous O rectangle. */
internal class ExportCallerIntent(
    val project: WorkbenchProject,
    val epoch: Long,
    val page: UInt?,
    val camera: Camera,
    val selection: ExportSelection,
    val binding: WorkflowBinding,
    val sourceOptions: ExportOptions,
    val options: ExportOptions,
    val geometry: ImageOutputExportGeometry,
) {
    fun matches(current: WorkbenchProject?, currentEpoch: Long, document: DocumentSnapshot?,
        info: ProjectInfo?, currentCamera: Camera, currentSelection: ExportSelection): Boolean =
        current === project && currentEpoch == epoch && currentSelection == selection &&
            (!selection.viewOnly || currentCamera == camera) && document != null && document.documentId == binding.documentId &&
            document.page == page && document.width == geometry.sourceWidth && document.height == geometry.sourceHeight &&
            document.render.revision.projectId == binding.projectId &&
            document.render.revision.hostSeq == binding.hostSeq && document.render.revision.stateHash == binding.stateHash &&
            info?.projectId == binding.projectId && info?.hostSeq == binding.hostSeq && info?.stateHash == binding.stateHash &&
            geometry.binding == binding && sourceOptions.documentId == binding.documentId &&
            options.documentId == binding.documentId && options.copy(region = sourceOptions.region) == sourceOptions &&
            (options.region?.let { region ->
                geometry.region?.let { projected -> region.x == projected.x.toDouble() &&
                    region.y == projected.y.toDouble() && region.width == projected.width.toDouble() &&
                    region.height == projected.height.toDouble() } == true
            } ?: (geometry.region == null))
}
