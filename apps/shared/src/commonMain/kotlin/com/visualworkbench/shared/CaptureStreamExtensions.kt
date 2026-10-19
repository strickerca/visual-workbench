package com.visualworkbench.shared

public expect suspend fun ProjectLink.openCaptureStream(memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCaptureStream
public expect suspend fun ProjectLink.openCapturePublisher(source: CapturePublicationSource,
    memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCapturePublisher
public expect suspend fun ProjectLink.captureRegionSignal():CaptureRegionSignal
public expect suspend fun ProjectLink.requestCaptureRegion(requestId:String):Unit
public expect suspend fun ProjectLink.resolveCaptureRegion(requestId:String,connectionEpoch:ULong,approve:Boolean):Unit
