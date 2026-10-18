package com.visualworkbench.shared

import kotlin.math.ceil
import kotlin.math.floor

/** Exact metadata fence used by both platform exporters. A missing cache is not
 * an identity frame: callers must explicitly load the committed output context. */
public fun ImageOutputSnapshot.matches(document: DocumentSnapshot): Boolean {
    val revision = document.render.revision
    return binding == WorkflowBinding(revision.projectId,document.documentId,revision.hostSeq,revision.stateHash) &&
        sourceWidth == document.width && sourceHeight == document.height && sourceBitDepth == document.bitDepth
}

public val ImageOutputSnapshot.outputWidth: UInt get() = output?.takeIf { it.enabled }?.settings?.outputWidth ?: sourceWidth
public val ImageOutputSnapshot.outputHeight: UInt get() = output?.takeIf { it.enabled }?.settings?.outputHeight ?: sourceHeight

/** D-edge → O-edge mapping of the canonical integral crop/quarter-turn/resize.
 * This is selection geometry only. All pixel/ICC/alpha work remains native.
 * At most four corners are admitted and every returned region is a positive
 * enclosing integer rectangle clipped to O; no silent full-image fallback. */
public fun imageOutputRegion(snapshot: ImageOutputSnapshot, documentCorners: List<Point>): Rect {
    if (documentCorners.size != 4 || documentCorners.any { !it.x.isFinite() || !it.y.isFinite() })
        throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val frame = snapshot.output?.takeIf { it.enabled }?.settings
    val mapped = if (frame == null) documentCorners else {
        if (frame.cropWidth == 0u || frame.cropHeight == 0u || frame.outputWidth == 0u || frame.outputHeight == 0u ||
            frame.cropX.toULong()+frame.cropWidth.toULong()>snapshot.sourceWidth.toULong() ||
            frame.cropY.toULong()+frame.cropHeight.toULong()>snapshot.sourceHeight.toULong()) throw WorkflowFailure(WorkflowFailureKind.Invalid)
        val w=frame.cropWidth.toDouble(); val h=frame.cropHeight.toDouble()
        val quarter=frame.clockwiseDegrees.toInt()
        val rw=if(quarter==90 || quarter==270) h else w
        val rh=if(quarter==90 || quarter==270) w else h
        documentCorners.map { point ->
            val x=point.x-frame.cropX.toDouble(); val y=point.y-frame.cropY.toDouble()
            val rotated=when(quarter){0->Point(x,y);90->Point(h-y,x);180->Point(w-x,h-y);270->Point(y,w-x)
                else->throw WorkflowFailure(WorkflowFailureKind.Invalid)}
            Point(rotated.x*frame.outputWidth.toDouble()/rw,rotated.y*frame.outputHeight.toDouble()/rh)
        }
    }
    if(mapped.any{!it.x.isFinite() || !it.y.isFinite()} || snapshot.outputWidth==0u || snapshot.outputHeight==0u)
        throw WorkflowFailure(WorkflowFailureKind.Invalid)
    val left=floor(mapped.minOf{it.x}).coerceIn(0.0,snapshot.outputWidth.toDouble())
    val top=floor(mapped.minOf{it.y}).coerceIn(0.0,snapshot.outputHeight.toDouble())
    val right=ceil(mapped.maxOf{it.x}).coerceIn(0.0,snapshot.outputWidth.toDouble())
    val bottom=ceil(mapped.maxOf{it.y}).coerceIn(0.0,snapshot.outputHeight.toDouble())
    if(right<=left || bottom<=top)throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return Rect(left,top,right-left,bottom-top)
}

/** A single complete display tile, never a resize or an export source. */
public fun imageOutputPreviewRegion(snapshot: ImageOutputSnapshot, x: UInt, y: UInt): SelectionRegion {
    if(x>=snapshot.outputWidth || y>=snapshot.outputHeight)throw WorkflowFailure(WorkflowFailureKind.Invalid)
    return SelectionRegion(x,y,minOf(512u,snapshot.outputWidth-x),minOf(512u,snapshot.outputHeight-y))
}
