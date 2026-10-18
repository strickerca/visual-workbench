package com.visualworkbench.shared

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlin.math.min

public data class MagnifierPane(public val rectangle:Rect,public val matrix:Transform,public val scale:Double,
    public val magnification:Int,public val smooth:Boolean,public val editable:Boolean)
public data class PrecisionFrame(public val lens:MagnifierPane?=null,public val loupe:MagnifierPane?=null,
    public val reticle:Point?=null,public val overview:List<Point> = emptyList())
public data class PrecisionState(public val frame:PrecisionFrame=PrecisionFrame(),public val inspection:Boolean=true,
    public val leftHanded:Boolean=false,public val smooth:Boolean=false,public val magnification:Int=4,
    public val sizeFraction:Double=.34,public val latched:Boolean=false,public val pending:Boolean=false,
    public val message:String?=null)
private data class LensPlacement(val rectangle:Rect,val focus:Point,val magnification:Int)
/** UI-thread-owned local view state. No project, link, edit, AI or send reference.
 * Base Camera is borrowed unchanged; all remapping uses canonical Rust geometry.
 * Each accepted/predicted batch is routed before entering the existing ink queue. */
public class PrecisionController(
    private val core:WorkbenchCore,private val changed:()->Unit,
    private val factory:(Camera)->WorkbenchPrecision=::precisionGeometry,
    private val renderMatrix:(Camera,Point,Point,Int)->Transform=::precisionViewTransform,
) {
    private val mutable:MutableStateFlow<PrecisionState> = MutableStateFlow(PrecisionState())
    public val state:StateFlow<PrecisionState> = mutable
    private var base:Camera?=null
    private var geometry:WorkbenchPrecision?=null
    private var placement:LensPlacement?=null
    private var queued:LensPlacement?=null
    private var hasQueued:Boolean=false
    private var pointer:Point?=null
    private var hovering:Boolean=false
    private var contact:Boolean=false
    private var peek:Boolean=false
    private var barrelAt:Long?=null
    private var draggedLoupe:Point?=null
    private var dead:Boolean=false

    /** Explicit view changes close the local lens. Never alter the supplied camera. */
    public fun synchronize(camera:Camera):Unit {
        check(!dead)
        if(base==camera)return
        check(!contact){"Cancel contact before changing the base view"}
        val next=factory(camera) // invalid new geometry must preserve the previous state
        try { geometry?.close() } catch(error:Exception) {next.close();throw error}
        geometry=next;base=camera;placement=null;queued=null;hasQueued=false;pointer=null;hovering=false;peek=false;barrelAt=null
        publish()
    }
    public fun frame():PrecisionFrame=state.value.frame
    public fun baseCamera():Camera?=base
    /** Exact D-to-physical matrix for snap radii/hit calibration. Capture with
     * the gesture; later UP may apply a queued view before its async edit runs. */
    public fun inputMatrix(point:Point?=null):Transform {
        val native=checkNotNull(geometry).mapping()
        val lens=if(contact)native.lensInput else point?.let{placement?.rectangle?.contains(it)==true}?:false
        return if(lens)native.lens else native.base
    }
    public fun inspection(value:Boolean):Unit {mutable.value=state.value.copy(inspection=value);publish()}
    public fun leftHanded(value:Boolean):Unit {mutable.value=state.value.copy(leftHanded=value);draggedLoupe=null;publish()}
    public fun smooth(value:Boolean):Unit {mutable.value=state.value.copy(smooth=value);publish()}
    public fun magnification(value:Int):Unit {
        require(value in listOf(2,4,8));mutable.value=state.value.copy(magnification=value)
        val desired=if(hasQueued)queued else placement
        if(desired!=null)request(desired.copy(magnification=value)) else publish()
    }
    public fun size(fraction:Double):Unit {
        require(fraction.isFinite()&&fraction in .18.. .60);mutable.value=state.value.copy(sizeFraction=fraction)
        val desired=if(hasQueued)queued else placement
        if(desired!=null){val camera=base?:return;val side=side(camera);request(desired.copy(rectangle=contained(Rect(desired.rectangle.x,desired.rectangle.y,side,side),camera)))} else publish()
    }
    public fun toggle():Unit {
        val camera=base?:return
        if((if(hasQueued)queued else placement)!=null){request(null);return}
        val target=pointer?:Point(camera.viewportWidth/2,camera.viewportHeight/2)
        val focus=core.mapPoints(camera,true,listOf(target)).single()
        val side=side(camera)
        val rect=contained(Rect((camera.viewportWidth-side)/2,(camera.viewportHeight-side)/2,side,side),camera)
        request(LensPlacement(rect,focus,state.value.magnification))
    }
    public fun moveLens(dx:Double,dy:Double):Unit {
        require(dx.isFinite()&&dy.isFinite());val camera=base?:return;val desired=(if(hasQueued)queued else placement)?:return
        request(desired.copy(rectangle=contained(desired.rectangle.copy(x=desired.rectangle.x+dx,y=desired.rectangle.y+dy),camera)))
    }
    public fun moveLoupe(dx:Double,dy:Double):Unit {
        require(dx.isFinite()&&dy.isFinite());val rect=state.value.frame.loupe?.rectangle?:return
        draggedLoupe=Point(rect.x+dx,rect.y+dy);publish()
    }
    public fun resetLoupePosition():Unit {draggedLoupe=null;publish()}
    private fun request(value:LensPlacement?):Unit {
        val native=geometry?:return
        if(value==null)native.closeLens() else native.configure(value.focus,center(value.rectangle),value.magnification)
        if(contact){queued=value;hasQueued=true}else placement=value
        publish()
    }
    /** Hover data is never sent to a drawing queue. Missing hover clears aim. */
    public fun hover(point:Point?):Unit {
        if(contact)return
        point?.let(::finite);pointer=point;hovering=point!=null
        if(point==null){barrelAt=null;peek=false}
        publish()
    }
    public fun down(point:Point):Unit {
        finite(point);check(!contact);val native=geometry?:error("Canvas geometry is unavailable")
        native.down(placement?.rectangle?.contains(point)==true);contact=true;hovering=false;pointer=point;publish()
    }
    public fun move(point:Point):Unit {finite(point);if(contact){pointer=point;publish()}}
    /** Call before end(): queued mapping never affects the final UP batch. */
    public fun route(points:List<Point>):List<Point> {
        require(points.size<=512);points.forEach(::finite)
        return geometry?.mapToBase(points,inside=points.firstOrNull()?.let{placement?.rectangle?.contains(it)==true}?:false)?:points
    }
    public fun end():Unit {
        if(contact){geometry?.up();contact=false;if(hasQueued){placement=queued;queued=null;hasQueued=false}}
        pointer=null;hovering=false;peek=false;barrelAt=null;publish()
    }
    public fun cancel():Unit=end()
    public fun reset():Unit {
        if(contact){geometry?.up();contact=false}
        geometry?.closeLens();placement=null;queued=null;hasQueued=false;pointer=null;hovering=false;peek=false;barrelAt=null;publish()
    }
    /** Generic barrel only: short release toggles; held button is render-only peek.
     * Timestamps come from the accepted input clock. No timer/provider is owned. */
    public fun barrel(down:Boolean,timeMs:Long):Unit {
        require(timeMs>=0)
        val prior=barrelAt
        if(down&&prior==null){barrelAt=timeMs;peek=true;publish()}
        else if(!down&&prior!=null){barrelAt=null;peek=false;if(timeMs>=prior&&timeMs-prior<=250)toggle()else publish()}
    }
    public fun close():Unit {
        if(dead)return
        geometry?.close();geometry=null;base=null;placement=null;queued=null;contact=false;dead=true
        mutable.value=PrecisionState();changed()
    }
    private fun side(camera:Camera):Double=min(512.0,min(camera.viewportWidth,camera.viewportHeight)*state.value.sizeFraction).coerceAtLeast(1.0)
    private fun pane(value:LensPlacement):MagnifierPane {
        val camera=checkNotNull(base)
        return MagnifierPane(value.rectangle,checkNotNull(geometry).mapping().lens,camera.scale*value.magnification,value.magnification,state.value.smooth,true)
    }
    private fun publish():Unit {
        val camera=base
        if(camera==null){mutable.value=state.value.copy(frame=PrecisionFrame());changed();return}
        val lens=placement?.let(::pane)
        val aim=pointer
        val loupe=if(aim!=null&&(contact||hovering)&&(state.value.inspection||peek)){
            val rectangle=inspectionRectangle(camera,aim,side(camera),state.value.leftHanded,draggedLoupe)
            rectangle?.let{
                val routed=geometry?.mapToBase(listOf(aim),placement?.rectangle?.contains(aim)==true)?:listOf(aim)
                val focus=core.mapPoints(camera,true,routed).single()
                MagnifierPane(it,renderMatrix(camera,focus,center(it),4),camera.scale*4,4,state.value.smooth,false)
            }
        }else null
        val overview=lens?.let{geometry?.viewToBase(corners(it.rectangle))}.orEmpty()
        mutable.value=state.value.copy(frame=PrecisionFrame(lens,loupe,aim,overview),latched=contact,pending=hasQueued,
            message=if(hasQueued)"Lens change queued until pen-up or cancellation." else if(aim!=null&&(contact||hovering)&&state.value.inspection&&loupe==null)"The viewport has no safe loupe position clear of the nib." else null)
        changed()
    }
}
private fun finite(p:Point):Unit {require(p.x.isFinite()&&p.y.isFinite())}
public fun Rect.contains(point:Point):Boolean=point.x>=x&&point.y>=y&&point.x<x+width&&point.y<y+height
private fun center(r:Rect):Point=Point(r.x+r.width/2,r.y+r.height/2)
private fun corners(r:Rect):List<Point> = listOf(Point(r.x,r.y),Point(r.x+r.width,r.y),Point(r.x+r.width,r.y+r.height),Point(r.x,r.y+r.height))
private fun contained(rect:Rect,camera:Camera):Rect {
    val width=rect.width.coerceIn(1.0,camera.viewportWidth);val height=rect.height.coerceIn(1.0,camera.viewportHeight)
    return Rect(rect.x.coerceIn(0.0,camera.viewportWidth-width),rect.y.coerceIn(0.0,camera.viewportHeight-height),width,height)
}
/** Pure placement: hide if no safe area exists; never cover the nib or overflow. */
public fun inspectionRectangle(camera:Camera,nib:Point,size:Double,leftHanded:Boolean,dragged:Point?=null):Rect? {
    finite(nib);require(size.isFinite()&&size>0)
    val gap=16.0
    val left=nib.x-size-gap;val right=nib.x+gap;val top=nib.y-size-gap;val bottom=nib.y+gap
    val xs=if(leftHanded)listOf(right,left)else listOf(left,right)
    val choices=listOfNotNull(dragged)+listOf(Point(xs[0],top),Point(xs[1],top),Point(xs[0],bottom),Point(xs[1],bottom))
    return choices.asSequence().map{contained(Rect(it.x,it.y,size,size),camera)}.firstOrNull{
        !Rect(it.x-gap,it.y-gap,it.width+gap*2,it.height+gap*2).contains(nib)
    }
}
