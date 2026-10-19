package com.visualworkbench.shared

public expect suspend fun ProjectLink.createCaptureDraft(stream: WorkbenchCaptureStream,
    documentId: String, layerId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCaptureDraft
