package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.util.Properties

enum class Tool { Select, Pan, Pen, Rectangle, Ellipse, Line, Arrow, Text, Callout }
enum class WindowMode { Windowed, Maximized, Fullscreen }
enum class Command(val label: String, val shortcut: String) {
    Import("Import file", "Ctrl+I"), Open("Open project", "Ctrl+O"), Export("Export image", "Ctrl+E"), Copy("Copy image…", "Ctrl+Shift+C"), Paste("Paste image…", "Ctrl+V"), Close("Close project", "Ctrl+W"), Quit("Quit", "Alt+F4"),
    Undo("Undo", "Ctrl+Z"), Redo("Redo", "Ctrl+Shift+Z"), Delete("Delete selection", "Delete"), SelectAll("Select all", "Ctrl+A"), ClearSelection("Clear selection", "Escape"),
    Select("Select tool", "V"), Pan("Pan tool", "H"), Pen("Pen tool", "P"), Rectangle("Rectangle tool", "R"), Ellipse("Ellipse tool", "E"), Line("Line tool", "L"), Arrow("Arrow tool", "A"), Text("Text tool", "T"), Callout("Numbered marker", "M"),
    Fit("Fit document", "F"), ActualPixels("Actual pixels", "Ctrl+1"), ZoomIn("Zoom in", "+"), ZoomOut("Zoom out", "-"), Windowed("Windowed", "Ctrl+Alt+1"), Maximized("Maximized", "Ctrl+Alt+2"), Fullscreen("Fullscreen", "F11"),
    FollowPeer("Follow peer", "Ctrl+Alt+F"), MatchPeer("Match peer view", "Ctrl+Alt+M"), PeerOutline("Peer viewport outline", "Ctrl+Alt+V"), Pairing("Pair devices", "Ctrl+P"), Settings("Settings", "Ctrl+,"), Diagnostics("Diagnostics", "F12"), Instructions("Edit instruction", "Enter"), CycleMarker("Next marker", "Tab"),
    Color("Next color", "C"), WidthUp("Increase width", "]"), WidthDown("Decrease width", "["), NudgeLeft("Nudge left (Shift=10)", "Left"), NudgeRight("Nudge right (Shift=10)", "Right"), NudgeUp("Nudge up (Shift=10)", "Up"), NudgeDown("Nudge down (Shift=10)", "Down")
}
data class SavedWindow(val mode: WindowMode=WindowMode.Windowed,val width:Float=1280f,val height:Float=820f,val x:Float?=null,val y:Float?=null)
data class LocationPolicy(val path:Path,val usesLocalAppData:Boolean,val reason:String)
object ProjectLocations {
    fun choose(documents:Path,localAppData:Path,cloudRoots:List<Path>):LocationPolicy {
        val normalized=documents.toAbsolutePath().normalize()
        val markers=setOf("onedrive","dropbox","google drive","googledrive","iclouddrive","box")
        val cloud=cloudRoots.any{normalized.startsWith(it.toAbsolutePath().normalize())} || normalized.any{part->markers.any{part.toString().lowercase().startsWith(it)}}
        return if(cloud) LocationPolicy(localAppData.resolve("Visual Workbench/Projects"),true,"Documents is in a cloud folder. Projects use local app storage to protect SQLite files.")
        else LocationPolicy(documents.resolve("Visual Workbench"),false,"Projects are stored in Documents, outside recognized cloud folders.")
    }
}
class DesktopPreferences(private val file:Path) {
    private val data=Properties()
    init { if(Files.exists(file)&&Files.size(file)<=64*1024) Files.newInputStream(file).use{data.load(it)} }
    fun window():SavedWindow {fun number(key:String,min:Float,max:Float,default:Float)=data.getProperty(key)?.toFloatOrNull()?.takeIf{it.isFinite()&&it in min..max}?:default
        val mode=WindowMode.entries.find{it.name==data.getProperty("window.mode")}?:WindowMode.Windowed
        return SavedWindow(mode,number("window.width",640f,8192f,1280f),number("window.height",480f,8192f,820f),data.getProperty("window.x")?.toFloatOrNull()?.takeIf{it.isFinite()&&it in -32000f..32000f},data.getProperty("window.y")?.toFloatOrNull()?.takeIf{it.isFinite()&&it in -32000f..32000f})
    }
    @Synchronized fun saveWindow(window:SavedWindow){data.setProperty("window.mode",window.mode.name);data.setProperty("window.width",window.width.toString());data.setProperty("window.height",window.height.toString());window.x?.let{data.setProperty("window.x",it.toString())};window.y?.let{data.setProperty("window.y",it.toString())};save()}
    @Synchronized fun device(core:WorkbenchCore):String = data.getProperty("device.id")?:core.newDeviceId().also{data.setProperty("device.id",it);save()}
    private fun save(){Files.createDirectories(file.parent);val temp=Files.createTempFile(file.parent,"vw-settings-",".tmp");try{Files.newOutputStream(temp).use{data.store(it,"Visual Workbench local preferences")};try{Files.move(temp,file,StandardCopyOption.REPLACE_EXISTING,StandardCopyOption.ATOMIC_MOVE)}catch(_:java.nio.file.AtomicMoveNotSupportedException){Files.move(temp,file,StandardCopyOption.REPLACE_EXISTING)}}finally{Files.deleteIfExists(temp)}}
}

/** Camera is local UI state. A document receipt updates only document fields. */
data class ViewState(val camera:Camera=Camera(Point(0.0,0.0),1.0,0.0,1.0,1.0),val followPeer:Boolean=false,val showPeerOutline:Boolean=false,val peer:Camera?=null) {
    fun receivePeerEdit():ViewState = this
    fun receivePeerView(value:Camera):ViewState = copy(peer=value,camera=if(followPeer) value.copy(viewportWidth=camera.viewportWidth,viewportHeight=camera.viewportHeight) else camera)
    fun matchPeer():ViewState = peer?.let{copy(camera=it.copy(viewportWidth=camera.viewportWidth,viewportHeight=camera.viewportHeight))}?:this
}

/** Geometry and gestures are scoped to an attachment, not just reused object IDs. */
data class RenderBinding(val epoch:Long,val projectId:String,val documentId:String,val hostSeq:ULong,val stateHash:String)
internal fun DocumentSnapshot.binding(epoch:Long):RenderBinding = RenderBinding(epoch,render.revision.projectId,documentId,render.revision.hostSeq,render.revision.stateHash)
internal enum class SnapshotDecision { Newer, Duplicate, Stale, Conflict }
internal fun snapshotDecision(current:DocumentSnapshot?,incoming:DocumentSnapshot,allowVisibleChange:Boolean=false):SnapshotDecision {
    if(current==null)return SnapshotDecision.Newer
    if(current.documentId!=incoming.documentId||current.render.revision.projectId!=incoming.render.revision.projectId)return SnapshotDecision.Conflict
    val before=current.render.revision;val after=incoming.render.revision
    return when {
        after.hostSeq<before.hostSeq->SnapshotDecision.Stale
        after.hostSeq>before.hostSeq->SnapshotDecision.Newer
        after.stateHash==before.stateHash->SnapshotDecision.Duplicate
        allowVisibleChange->SnapshotDecision.Newer
        else->SnapshotDecision.Conflict
    }
}
internal fun geometryMatches(binding:RenderBinding?,state:EditorState):Boolean = binding!=null&&state.renderIssue==null&&state.document?.binding(state.projectEpoch)==binding

/** Ordinary editor shortcuts belong to the canvas. Dialog text and focused
 * controls keep Tab/Enter/arrows; modified app commands and F-keys stay global. */
internal fun shortcutAllowed(canvasFocused:Boolean,modalText:Boolean,modified:Boolean,functionKey:Boolean):Boolean =
    !modalText&&(canvasFocused||modified||functionKey)

fun translated(value:Transform,dx:Double,dy:Double):Transform=value.copy(e=value.e+dx,f=value.f+dy)
fun scaledAbout(value:Transform,origin:Point,sx:Double,sy:Double):Transform = Transform(value.a*sx,value.b*sy,value.c*sx,value.d*sy,origin.x+(value.e-origin.x)*sx,origin.y+(value.f-origin.y)*sy)
fun union(rectangles:List<Rect>):Rect? {if(rectangles.isEmpty())return null;val left=rectangles.minOf{it.x};val top=rectangles.minOf{it.y};return Rect(left,top,rectangles.maxOf{it.x+it.width}-left,rectangles.maxOf{it.y+it.height}-top)}
fun previewBounds(item:RenderItem,preview:Transform?):Rect {
    if(preview==null)return item.bounds
    val a=item.transform;val determinant=a.a*a.d-a.b*a.c;if(kotlin.math.abs(determinant)<1e-18)return item.bounds
    val r=item.bounds;val points=listOf(Point(r.x,r.y),Point(r.x+r.width,r.y),Point(r.x+r.width,r.y+r.height),Point(r.x,r.y+r.height)).map{p->val x=p.x-a.e;val y=p.y-a.f;val localX=(a.d*x-a.c*y)/determinant;val localY=(-a.b*x+a.a*y)/determinant;Point(preview.a*localX+preview.c*localY+preview.e,preview.b*localX+preview.d*localY+preview.f)}
    val x=points.minOf{it.x};val y=points.minOf{it.y};return Rect(x,y,points.maxOf{it.x}-x,points.maxOf{it.y}-y)
}
