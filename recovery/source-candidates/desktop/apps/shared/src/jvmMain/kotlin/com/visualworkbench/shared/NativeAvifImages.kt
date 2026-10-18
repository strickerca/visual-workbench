package com.visualworkbench.shared

public actual fun inspectAvifImage(original:ByteArray,memoryBudgetBytes:ULong):AvifImageInfo {
    prepareCoreLibrary()
    try { val result=com.visualworkbench.bindings.core.inspectAvifImage(original,memoryBudgetBytes)
        return AvifImageInfo(result.sourceAssetId,result.width,result.height,result.bitDepth,result.orientation,result.iccProfile,result.nclx?.let{AvifColorInfo(it.primaries,it.transfer,it.matrix,it.fullRange)},result.estimatedPeakBytes)
    }catch(error:com.visualworkbench.bindings.core.AvifImageException){throw AvifImageFailure(when(error){
        is com.visualworkbench.bindings.core.AvifImageException.Invalid->AvifImageFailureKind.Invalid
        is com.visualworkbench.bindings.core.AvifImageException.Unavailable->AvifImageFailureKind.Unavailable
        is com.visualworkbench.bindings.core.AvifImageException.Unsupported->AvifImageFailureKind.Unsupported
        is com.visualworkbench.bindings.core.AvifImageException.Depth->AvifImageFailureKind.Depth
        is com.visualworkbench.bindings.core.AvifImageException.Color->AvifImageFailureKind.Color
        is com.visualworkbench.bindings.core.AvifImageException.Orientation->AvifImageFailureKind.Orientation
        is com.visualworkbench.bindings.core.AvifImageException.Memory->AvifImageFailureKind.Memory
        is com.visualworkbench.bindings.core.AvifImageException.Busy->AvifImageFailureKind.Busy
        is com.visualworkbench.bindings.core.AvifImageException.Cancelled->AvifImageFailureKind.Cancelled
        is com.visualworkbench.bindings.core.AvifImageException.Decode->AvifImageFailureKind.Decode
    })}
}
