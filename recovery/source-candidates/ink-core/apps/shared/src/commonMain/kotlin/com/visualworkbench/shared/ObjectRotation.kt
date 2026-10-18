package com.visualworkbench.shared

import kotlin.math.abs
import kotlin.math.sqrt

/** These inputs are document coordinates. The native pinned-libm solver returns
 * each immutable original left-multiplied by one common pivot rotation. It does
 * not alter a project, sample list, outline, style, mask or original image. */
public expect fun objectRotationTransforms(originals: List<Transform>, pivot: Point,
    first: Point, current: Point, snap15: Boolean): List<Transform>

public data class ObjectRotationFrame(public val binding: WorkflowBinding,
    public val attachment: Long, public val camera: Camera)
public fun DocumentSnapshot.objectRotationFrame(attachment: Long, camera: Camera): ObjectRotationFrame =
    ObjectRotationFrame(WorkflowBinding(render.revision.projectId,documentId,render.revision.hostSeq,render.revision.stateHash),attachment,camera)

/** Physical-pixel presentation/hit geometry. It is never persisted/exported. */
public data class ObjectRotationHandle(public val center: Point, public val anchor: Point, public val knob: Point) {
    public fun hit(pointer: Point, radiusPixels: Double): Boolean {
        if (!physical(pointer) || !radiusPixels.isFinite() || radiusPixels <= 0.0 || radiusPixels > 4096.0) return false
        val x=pointer.x-knob.x;val y=pointer.y-knob.y
        return x*x+y*y <= radiusPixels*radiusPixels
    }
}
/** corners are clockwise physical top-left/top-right/bottom-right/bottom-left,
 * including the current camera and preview. The outward stem has fixed physical
 * size regardless of document zoom. A flat rectangle uses its upper normal. */
public fun objectRotationHandle(corners: List<Point>, offsetPixels: Double): ObjectRotationHandle? {
    if (corners.size != 4 || corners.any { !physical(it) } || !offsetPixels.isFinite() || offsetPixels <= 0.0 || offsetPixels > 4096.0) return null
    val center=Point(corners.sumOf{it.x/4.0},corners.sumOf{it.y/4.0})
    val anchor=Point(corners[0].x/2.0+corners[1].x/2.0,corners[0].y/2.0+corners[1].y/2.0)
    var x=anchor.x-center.x;var y=anchor.y-center.y
    if(x==0.0&&y==0.0){x=corners[1].y-corners[0].y;y=corners[0].x-corners[1].x}
    val distance=sqrt(x*x+y*y)
    if(!distance.isFinite()||distance==0.0)return null
    val knob=Point(anchor.x+x/distance*offsetPixels,anchor.y+y/distance*offsetPixels)
    return if(physical(knob))ObjectRotationHandle(center,anchor,knob)else null
}
private fun physical(point:Point):Boolean=point.x.isFinite()&&point.y.isFinite()&&abs(point.x)<=1e12&&abs(point.y)<=1e12

public fun objectRotationAllowed(items: List<RenderItem>): Boolean = items.size in 1..256 &&
    items.none { it.locked || it.shape is Shape.Guide || it.shape is Shape.Adjustment } &&
    items.map { it.objectId }.distinct().size == items.size

/** One UI-thread contact. Every real sample recomputes from the pen-down
 * originals; there is no incremental matrix drift. A stale camera, attachment,
 * document or visible revision seals the contact. finish returns commands at
 * most once, and the existing editor's CAS transaction remains the commit owner. */
public class ObjectRotationContact private constructor(
    private val frame: ObjectRotationFrame,
    private val pivot: Point,
    private val first: Point,
    private val originals: List<Pair<String,Transform>>,
    private val solve: (List<Transform>,Point,Point,Point,Boolean)->List<Transform>,
) {
    private var ended: Boolean=false
    private var latest: Map<String,Transform> = originals.toMap()
    public fun preview(point: Point, current: ObjectRotationFrame, snap15: Boolean): Map<String,Transform>? {
        if(ended)return null
        if(current!=frame){cancel();return null}
        try {
            val result=solve(originals.map{it.second},pivot,first,point,snap15)
            if(result.size!=originals.size)throw CoreFailure(CoreFailureKind.Invalid)
            latest=originals.indices.associate{originals[it].first to result[it]}
            return latest
        }catch(error:Exception){cancel();throw error}
    }
    public fun finish(point: Point?, current: ObjectRotationFrame, snap15: Boolean): List<EditCommand>? {
        if(ended)return null
        if(current!=frame){cancel();return null}
        if(point!=null && preview(point,current,snap15)==null)return null
        ended=true
        return originals.mapNotNull{(id,before)->latest[id]?.takeIf{it!=before}?.let{EditCommand.SetTransform(id,it)}}
    }
    public fun cancel(): Unit { ended=true;latest=emptyMap() }
    public companion object {
        public fun begin(frame: ObjectRotationFrame, items: List<RenderItem>, pivot: Point, first: Point,
            solve: (List<Transform>,Point,Point,Point,Boolean)->List<Transform> = ::objectRotationTransforms): ObjectRotationContact? {
            if(!objectRotationAllowed(items)||!physical(pivot)||!physical(first)||pivot==first)return null
            val values=items.map{it.objectId to it.transform}
            // Preflight the bounded canonical affine inputs without movement.
            if(solve(values.map{it.second},pivot,first,first,false).size!=values.size)throw CoreFailure(CoreFailureKind.Invalid)
            return ObjectRotationContact(frame,pivot,first,values,solve)
        }
    }
}
