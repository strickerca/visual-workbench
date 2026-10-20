package com.visualworkbench.shared

/** Exact locally approved physical source. These DTOs never originate in a
 * peer announcement and never resolve labels into OS capture authority. */
public data class CaptureLocalRect(public val x:Int,public val y:Int,public val width:UInt,public val height:UInt)
public data class CaptureLocalWindow(public val window:ULong,public val processId:UInt,public val processCreated:ULong,
    public val client:CaptureLocalRect,public val frame:CaptureLocalRect,public val dpi:UInt,public val observedNs:ULong)
public data class CaptureLocalMonitor(public val monitor:ULong,public val device:String,public val physical:CaptureLocalRect,
    public val dpi:UInt,public val rotation:UInt,public val primary:Boolean,public val observedNs:ULong)
public sealed interface CaptureLocalSelection {
    public data class Window(public val target:CaptureLocalWindow):CaptureLocalSelection
    public data class Monitor(public val target:CaptureLocalMonitor):CaptureLocalSelection
    public data class Region(public val target:CaptureLocalMonitor,public val region:CaptureLocalRect):CaptureLocalSelection
}
public data class CaptureLocalOptions(public val explicitOwnerAction:Boolean,public val selection:CaptureLocalSelection,
    public val helperPath:String,public val trustedHelperSha256:String,public val ringBytes:ULong=300_000_000uL)
public enum class CaptureLocalProgressKind { Idle, Tiles, OriginalProvided, OriginalUnavailable }
public data class CaptureLocalProgress(public val kind:CaptureLocalProgressKind,public val frameId:ULong=0uL,public val admitted:Boolean=false)
public interface WorkbenchLocalCapture {
    /** One bounded helper exchange and authenticated native admission. Frozen
     * sessions inspect source liveness without acquiring additional frames. */
    public suspend fun pumpOnce():CaptureLocalProgress
    public fun cancel():Unit
    /** Completes only after actual process and worker retirement. */
    public suspend fun close():Unit
}
public expect suspend fun WorkbenchCapturePublisher.openLocalCapture(options:CaptureLocalOptions):WorkbenchLocalCapture
