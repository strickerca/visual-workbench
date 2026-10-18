package com.visualworkbench.shared

import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.DisplayColorException
import com.visualworkbench.bindings.core.DisplayColorRequest as NRequest
import com.visualworkbench.bindings.core.DisplayColorSpace as NSpace
import com.visualworkbench.bindings.core.DisplayRegion as NRegion
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

internal suspend fun originalColorsNative(handle:ProjectSession,request:DisplayColorRequest):DisplayColorPixels {
    currentCoroutineContext().ensureActive()
    return settledWorkflow { cancel ->
        val result=try{handle.displayOriginal(NRequest(workflowBinding(request.binding),request.region?.let{NRegion(it.x,it.y,it.width,it.height)},when(request.space){DisplayColorSpace.Srgb->NSpace.SRGB;DisplayColorSpace.DisplayP3->NSpace.DISPLAY_P3},request.assumeUntaggedSrgb,request.allowDisplayDepthReduction,request.memoryBudgetBytes),cancel)}
        catch(error:DisplayColorException){throw DisplayColorFailure(when(error){
            is DisplayColorException.Invalid->DisplayColorFailureKind.Invalid
            is DisplayColorException.Stale->DisplayColorFailureKind.Stale
            is DisplayColorException.Untagged->DisplayColorFailureKind.Untagged
            is DisplayColorException.Unsupported->DisplayColorFailureKind.Unsupported
            is DisplayColorException.Depth->DisplayColorFailureKind.Depth
            is DisplayColorException.Memory->DisplayColorFailureKind.Memory
            is DisplayColorException.Cancelled->DisplayColorFailureKind.Cancelled
            is DisplayColorException.Closed->DisplayColorFailureKind.Closed
            is DisplayColorException.Backpressure->DisplayColorFailureKind.Backpressure
            is DisplayColorException.Storage->DisplayColorFailureKind.Storage
        },(error as? DisplayColorException.Memory)?.estimated?:0uL,(error as? DisplayColorException.Memory)?.budget?:0uL,error)}
        DisplayColorPixels(workflowBinding(result.binding),result.sourceAssetId,DisplayRegion(result.region.x,result.region.y,result.region.width,result.region.height),when(result.space){NSpace.SRGB->DisplayColorSpace.Srgb;NSpace.DISPLAY_P3->DisplayColorSpace.DisplayP3},result.rgba,result.iccProfile,result.sourceBitDepth,result.assumedSrgb,result.depthReduced,result.estimatedPeakBytes).also{validateDisplayPixels(it,request)}
    }
}
