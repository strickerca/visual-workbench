package com.visualworkbench.shared

public actual fun inspectOsImage(original:ByteArray,memoryBudgetBytes:ULong):OsImageInfo {
    prepareCoreLibrary()
    try { val result=com.visualworkbench.bindings.core.inspectOsImage(original,memoryBudgetBytes)
        return OsImageInfo(result.sourceAssetId,result.width,result.height,result.bitDepth,result.orientation,result.iccProfile,result.estimatedPeakBytes)
    }catch(error:com.visualworkbench.bindings.core.OsImageException){throw OsImageFailure(when(error){
        is com.visualworkbench.bindings.core.OsImageException.Invalid->OsImageFailureKind.Invalid
        is com.visualworkbench.bindings.core.OsImageException.Unavailable->OsImageFailureKind.Unavailable
        is com.visualworkbench.bindings.core.OsImageException.Unsupported->OsImageFailureKind.Unsupported
        is com.visualworkbench.bindings.core.OsImageException.Depth->OsImageFailureKind.Depth
        is com.visualworkbench.bindings.core.OsImageException.Color->OsImageFailureKind.Color
        is com.visualworkbench.bindings.core.OsImageException.Orientation->OsImageFailureKind.Orientation
        is com.visualworkbench.bindings.core.OsImageException.Memory->OsImageFailureKind.Memory
        is com.visualworkbench.bindings.core.OsImageException.Busy->OsImageFailureKind.Busy
        is com.visualworkbench.bindings.core.OsImageException.Cancelled->OsImageFailureKind.Cancelled
        is com.visualworkbench.bindings.core.OsImageException.Decode->OsImageFailureKind.Decode
    })}
}
