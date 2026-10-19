package com.visualworkbench.shared

import kotlin.math.abs

public data class ObjectRotationFrame(public val binding: WorkflowBinding,
    public val attachment: Long, public val camera: Camera, public val page: UInt? = null)
public fun DocumentSnapshot.objectRotationFrame(attachment: Long, camera: Camera): ObjectRotationFrame =
    ObjectRotationFrame(WorkflowBinding(render.revision.projectId,documentId,render.revision.hostSeq,render.revision.stateHash),attachment,camera,page)

public fun objectRotationAllowed(items: List<RenderItem>): Boolean = items.size in 1..256 &&
    items.none { it.locked || it.shape is Shape.Guide || it.shape is Shape.Adjustment } &&
    items.map { it.objectId }.distinct().size == items.size

/** Inject the geometry owner's solver; this contact adds no affine engine or
 * renderer. One UI-thread contact. Every real sample recomputes from the pen-down
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
            if(!physical(point))throw CoreFailure(CoreFailureKind.Invalid)
            val result=solve(originals.map{it.second},pivot,first,point,snap15)
            if(result.size!=originals.size||result.any{!finite(it)})throw CoreFailure(CoreFailureKind.Invalid)
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
            solve: (List<Transform>,Point,Point,Point,Boolean)->List<Transform>): ObjectRotationContact? {
            if(!objectRotationAllowed(items)||!physical(pivot)||!physical(first)||pivot==first)return null
            val values=items.map{it.objectId to it.transform}
            // Preflight the bounded canonical affine inputs without movement.
            val checked=solve(values.map{it.second},pivot,first,first,false)
            if(checked.size!=values.size||checked.any{!finite(it)})throw CoreFailure(CoreFailureKind.Invalid)
            return ObjectRotationContact(frame,pivot,first,values,solve)
        }
    }
}

private fun physical(point:Point):Boolean=point.x.isFinite()&&point.y.isFinite()&&abs(point.x)<=1e12&&abs(point.y)<=1e12
private fun finite(value:Transform):Boolean=listOf(value.a,value.b,value.c,value.d,value.e,value.f).all{it.isFinite()}
