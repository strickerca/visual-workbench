package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CreateFileImageProject
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.CaptureImportDescriptor as NCapture
import com.visualworkbench.bindings.core.createCaptureProject as createNativeCapture
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

public actual suspend fun createCaptureProject(options:CreateFileProject,capture:CaptureImportDescriptor):WorkbenchProject {
    val native=NCapture(capture.explicitOwnerAction,capture.captureSessionId,capture.frameId,capture.geometryRevision,
        capture.platform,capture.sourceKind,capture.physicalX,capture.physicalY,capture.width,capture.height,
        capture.dpiScale,capture.monotonicTimestampNs,capture.capturedAtMs,capture.windowHandle,capture.monitorId,capture.expectedSourceAssetId)
    val handle=settledStream(release={value:ProjectSession->try{value.closeSession()}finally{value.destroy()}}){cancel->
        createNativeCapture(CreateFileImageProject(options.path,options.projectId,options.documentId,options.layerId,options.deviceId,options.title,
            options.sourcePath,options.workDirectory,options.nowMs,options.memoryBudgetBytes,options.maxEncodedBytes,options.maxScratchBytes),native,cancel)
    }
    try{return wrapNativeProject(handle)}catch(error:Throwable){withContext(NonCancellable){try{handle.closeSession()}finally{handle.destroy()}};throw error}
}
