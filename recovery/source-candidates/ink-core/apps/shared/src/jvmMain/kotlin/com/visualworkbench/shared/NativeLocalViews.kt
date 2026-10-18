package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Camera as NCamera
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.LocalViewsBinding as NBinding
import com.visualworkbench.bindings.core.LocalViewsMutation as NMutation
import com.visualworkbench.bindings.core.LocalViewsSnapshot as NSnapshot
import com.visualworkbench.bindings.core.ProjectSession

public actual fun localViewWorkflows(project:WorkbenchProject):WorkbenchLocalViews=NativeLocalViews(nativeProjectHandle(project))
private fun LocalViewsBinding.native():NBinding {
    if(projectId.length!=36||documentId.length!=36||deviceId.length!=26)throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return NBinding(projectId,documentId,deviceId)
}
private fun Camera.native():NCamera {
    if(!validLocalCamera(this))throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return NCamera(NPoint(center.x,center.y),scale,rotation,viewportWidth,viewportHeight)
}
private fun NCamera.common():Camera=Camera(Point(center.x,center.y),scale,rotation,viewportWidth,viewportHeight)
private fun NSnapshot.common():LocalViewsSnapshot=LocalViewsSnapshot(LocalViewsBinding(binding.projectId,binding.documentId,binding.deviceId),revision,
    camera?.common(),bookmarks.map{RegionBookmark(it.id,it.name,it.camera.common())})
private class NativeLocalViews(private val handle:ProjectSession):WorkbenchLocalViews {
    override suspend fun snapshot(binding:LocalViewsBinding,memoryBudgetBytes:ULong):LocalViewsSnapshot {
        val captured=binding.native()
        return settledWorkflow{token->handle.localViewsSnapshot(captured,memoryBudgetBytes,token).common()}
    }
    override suspend fun change(binding:LocalViewsBinding,expectedRevision:ULong,operationId:String,
        mutation:LocalViewsMutation,memoryBudgetBytes:ULong):LocalViewsSnapshot {
        if(operationId.length!=36)throw WorkflowFailure(WorkflowFailureKind.Invalid)
        fun id(value:String):String {if(value.length!=36)throw WorkflowFailure(WorkflowFailureKind.Invalid);return value}
        fun name(value:String):String {if(!validBookmarkName(value))throw WorkflowFailure(WorkflowFailureKind.Invalid);return value}
        val owned=when(mutation){
            is LocalViewsMutation.Remember->NMutation.Remember(mutation.camera.native())
            is LocalViewsMutation.AddBookmark->NMutation.AddBookmark(id(mutation.id),name(mutation.name),mutation.camera.native())
            is LocalViewsMutation.RenameBookmark->NMutation.RenameBookmark(id(mutation.id),name(mutation.name))
            is LocalViewsMutation.DeleteBookmark->NMutation.DeleteBookmark(id(mutation.id))
        }
        val captured=binding.native()
        return settledWorkflow{token->handle.changeLocalViews(captured,expectedRevision,operationId,owned,memoryBudgetBytes,token).common()}
    }
}
