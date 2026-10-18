package com.visualworkbench.shared

public enum class DisplayColorSpace { Srgb, DisplayP3 }
public data class DisplayRegion(public val x:UInt,public val y:UInt,public val width:UInt,public val height:UInt)
public data class DisplayColorRequest(
    public val binding:WorkflowBinding,
    public val region:DisplayRegion?=null,
    public val space:DisplayColorSpace=DisplayColorSpace.Srgb,
    public val assumeUntaggedSrgb:Boolean=false,
    public val allowDisplayDepthReduction:Boolean=false,
    public val memoryBudgetBytes:ULong=256uL*1024uL*1024uL,
)
/** Straight-alpha, target-space RGBA8. Never an original or export input. */
public data class DisplayColorPixels(
    public val binding:WorkflowBinding,public val sourceAssetId:String,
    public val region:DisplayRegion,public val space:DisplayColorSpace,
    public val rgba:ByteArray,public val iccProfile:ByteArray,public val sourceBitDepth:UByte,
    public val assumedSrgb:Boolean,public val depthReduced:Boolean,public val estimatedPeakBytes:ULong,
)
public enum class DisplayColorFailureKind { Invalid, Stale, Untagged, Unsupported, Depth, Memory, Cancelled, Closed, Backpressure, Storage }
public class DisplayColorFailure(public val kind:DisplayColorFailureKind,public val estimated:ULong=0uL,public val budget:ULong=0uL,cause:Throwable?=null):Exception("Display color: $kind",cause)
/** Optional additive capability; older facades retain their explicit sRGB path. */
public interface WorkbenchColorDisplay {
    public suspend fun originalColors(request:DisplayColorRequest):DisplayColorPixels
}
public fun originalDisplayRequest(document: DocumentSnapshot, space: DisplayColorSpace, assumeSrgb: Boolean, allowDepth: Boolean): DisplayColorRequest {
    val revision = document.render.revision
    return DisplayColorRequest(WorkflowBinding(revision.projectId, document.documentId, revision.hostSeq, revision.stateHash),
        DisplayRegion(0u, 0u, document.width, document.height), space, assumeSrgb, allowDepth)
}
public const val MAX_COLOR_DISPLAY_PIXELS: ULong = 4_194_304uL

/** Validated against the owned request, never against the response's own ticket.
 * Platforms use the fixed target-space tag; they do not reinterpret source ICC.
 * The source original and every export retain independent depth/color choices. */
public fun validateDisplayPixels(value: DisplayColorPixels, request: DisplayColorRequest): Unit {
    val n = value.region.width.toULong() * value.region.height.toULong()
    val depth = value.sourceBitDepth.toInt()
    val regionMatches = request.region?.let { it == value.region }
        ?: (value.region.x == 0u && value.region.y == 0u)
    if (value.binding != request.binding || value.space != request.space || !regionMatches ||
        value.sourceAssetId.length != 64 || value.sourceAssetId.any { it !in "0123456789abcdef" } ||
        value.region.width == 0u || value.region.height == 0u || n > MAX_COLOR_DISPLAY_PIXELS ||
        value.region.x.toULong() + value.region.width.toULong() > UInt.MAX_VALUE.toULong() ||
        value.region.y.toULong() + value.region.height.toULong() > UInt.MAX_VALUE.toULong() ||
        n * 4uL != value.rgba.size.toULong() || value.iccProfile.size !in 132..4 * 1024 * 1024 ||
        depth !in listOf(8, 16) || value.depthReduced != (depth > 8) ||
        (value.depthReduced && !request.allowDisplayDepthReduction) ||
        (value.assumedSrgb && !request.assumeUntaggedSrgb) ||
        value.estimatedPeakBytes == 0uL || value.estimatedPeakBytes > request.memoryBudgetBytes)
        throw DisplayColorFailure(DisplayColorFailureKind.Invalid)
}

/** Main/UI-thread admission. Latest request, project lifetime, full revision and
 * window color choice must all still match before any platform image is adopted. */
public class DisplayAdoption {
    private var sequence: ULong = 0uL
    public fun issue(epoch: Long, request: DisplayColorRequest): DisplayAdoptionTicket {
        sequence += 1uL
        return DisplayAdoptionTicket(sequence, epoch, request)
    }
    public fun invalidate(): Unit { sequence += 1uL }
    public fun accepts(ticket: DisplayAdoptionTicket, epoch: Long, binding: WorkflowBinding?, space: DisplayColorSpace): Boolean =
        ticket.sequence == sequence && ticket.epoch == epoch && ticket.request.binding == binding && ticket.request.space == space
}
public data class DisplayAdoptionTicket internal constructor(
    internal val sequence: ULong,
    public val epoch: Long,
    public val request: DisplayColorRequest,
)
