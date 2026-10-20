@file:OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
package com.visualworkbench.desktop

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.visualworkbench.desktop.stream.DesktopStreamCanvas
import com.visualworkbench.shared.*
import kotlinx.coroutines.*

private enum class StreamDecision { Close, Resume, Degraded }

/** Explicit local receiver UI. Parent commands remain fenced until the complete
 * receiver chain retires. Disposing this panel never claims cleanup succeeded. */
@Composable internal fun DesktopStreamPanel(controller:EditorController,root:DesktopStreamRootState){
    val scope=rememberCoroutineScope()
    val session=root.session
    fun close(){scope.launch{try{controller.closeCaptureStream()}catch(_:Exception){controller.report("Stream cleanup is pending. Retry before leaving this project.")}}}
    if(session==null){
        Column(Modifier.fillMaxSize().background(Color(0xff101720)).padding(24.dp)){
            Text(if(root.opening)"Opening paired live capture…" else "Capture cleanup pending")
            root.issue?.let{Text(it)}
            Button(onClick=::close){Text(if(root.opening)"Cancel and close" else "Retry cleanup")}
        }
        return
    }
    val state by session.state.collectAsState()
    val parentState by controller.state.collectAsState()
    val annotation by session.annotation.state.collectAsState()
    val view=state.view?.view
    val viewState=view?.state?.collectAsState()?.value
    val adapter=state.child?.adapter
    val childFlow=runCatching{adapter?.state}.getOrNull()
    val childState=childFlow?.collectAsState()?.value
    val density=LocalDensity.current.density.toDouble()
    val focused=LocalWindowInfo.current.isWindowFocused
    var size by remember(session){mutableStateOf(IntSize.Zero)}
    var decision by remember(session){mutableStateOf<StreamDecision?>(null)}
    var now by remember(session){mutableStateOf(System.currentTimeMillis())}
    val latestState by rememberUpdatedState(state)
    val latestAnnotation by rememberUpdatedState(annotation)
    val latestDecision by rememberUpdatedState(decision)
    val latestFocused by rememberUpdatedState(focused)
    val latestAdapter by rememberUpdatedState(adapter)
    val input=remember(session){DesktopStreamContactInput(scope,{latestAdapter},
        admit={latestFocused&&latestDecision==null&&!latestState.closing&&!latestState.cleanupPending&&
            !latestAnnotation.saveRetry&&latestAnnotation.saved==null&&!latestAnnotation.editorFailed},
        freeze={point,camera->session.freeze(point,camera)?.adapter},report=controller::report)}
    LaunchedEffect(focused,decision){if(!focused||decision!=null)input.cancel()}
    SideEffect{input.viewport(size.width.toDouble(),size.height.toDouble(),density)}
    DisposableEffect(input){onDispose{input.cancel()}}
    LaunchedEffect(session){while(isActive){now=System.currentTimeMillis();delay(250)}}
    LaunchedEffect(view,childState?.view?.camera,density){
        if(view!=null)view.setCamera(childState?.view?.camera?.let{streamAwtCamera(it,density)})
    }
    fun runAction(action:suspend ()->Unit){
        if(input.busy||state.busy||state.closing)return
        input.cancel()
        scope.launch{try{action()}catch(cancel:CancellationException){throw cancel}
            catch(_:Exception){controller.report("The stream operation did not finish. Keep this view open and use its retry controls.")}}
    }
    fun resume(){runAction{session.resume()}}
    val frozen=annotation.frozen!=null
    val locked=state.busy||state.closing||root.cleanupPending||input.busy
    val age=session.annotation.estimatedFrameAgeMs(now)?.let{"~${(it/1000).toLong()} s old (clock estimate)"}?:"age unavailable"
    StreamPanelLayout(Modifier.fillMaxSize().background(Color(0xff101720)),header={
        Row(Modifier.fillMaxWidth().height(StreamPanelHeaderHeight).padding(8.dp),verticalAlignment=Alignment.CenterVertically){
            Column(Modifier.weight(1f)){
                Text("${if(frozen)"FROZEN" else when(state.signal.stage){CaptureStreamStage.Live->"LIVE";CaptureStreamStage.AwaitingSource->"WAITING";CaptureStreamStage.Freezing->"FREEZING";else->"SOURCE CLOSED"}} · ${state.signal.sourceName?:"Paired capture source"}",maxLines=1)
                Text(if(frozen)"$age · "+when{
                    annotation.saved!=null->if(annotation.degradedSaved)"Saved with explicit degraded consent" else "Saved losslessly"
                    annotation.lossless&&viewState?.lossless==true->"Lossless original displayed"
                    annotation.lossless->"Lossless original verified; display update pending"
                    state.signal.stage==CaptureStreamStage.FrozenDegraded->"Original unavailable · degraded preview"
                    else->"Waiting for the exact original"
                } else "Touch the displayed source with the pen or primary mouse button to annotate.",maxLines=2)
            }
            if(!frozen)TextButton(enabled=!locked,onClick={runAction{session.requestComputerRegion()}}){Text("Request region")}
            TextButton(enabled=(!locked||root.cleanupPending||state.cleanupPending)&&!annotation.saveRetry,onClick={
                if(annotation.draft!=null&&annotation.saved==null)decision=StreamDecision.Close else close()
            }){Text(if(root.cleanupPending||state.cleanupPending)"Retry cleanup" else "Close")}
        }
    },issue={
        Box(Modifier.fillMaxWidth().height(StreamPanelIssueHeight)){
            (root.issue?:state.issue?:annotation.failure?:viewState?.failure?:parentState.message)?.let{Text(it,Modifier.padding(horizontal=12.dp),color=Color(0xffe8bb75),maxLines=2)}
        }
    },tools={
        Row(Modifier.fillMaxWidth().height(StreamPanelToolbarHeight).horizontalScroll(rememberScrollState()).padding(horizontal=8.dp),verticalAlignment=Alignment.CenterVertically){
            if(frozen){
                val editing=!locked&&!annotation.saveRetry&&annotation.saved==null&&!annotation.editorFailed
                for(tool in listOf(Tool.Pen,Tool.Select,Tool.Pan,Tool.Rectangle,Tool.Ellipse,Tool.Line,Tool.Arrow,Tool.Text)){
                    TextButton(enabled=editing&&adapter!=null,onClick={adapter?.controller?.tool(tool)}){Text(tool.name)}
                }
                TextButton(enabled=editing&&adapter!=null,onClick={adapter?.controller?.chooseEraser(EraserChoice.Object)}){Text("Erase")}
                TextButton(enabled=editing&&adapter!=null,onClick={adapter?.controller?.undo(false)}){Text("Undo")}
                Spacer(Modifier.weight(1f))
                Button(enabled=!locked&&adapter!=null&&!annotation.editorFailed&&annotation.saved==null&&
                    (annotation.lossless||annotation.saveRetry),onClick={runAction{session.save()}}){Text(if(annotation.saveRetry)"Retry same Save" else "Save")}
                if(!annotation.lossless&&state.signal.stage==CaptureStreamStage.FrozenDegraded)
                    TextButton(enabled=!locked&&adapter!=null&&!annotation.editorFailed,onClick={decision=StreamDecision.Degraded}){Text("Save degraded…")}
                TextButton(enabled=!locked&&!annotation.saveRetry,onClick={if(annotation.saved==null)decision=StreamDecision.Resume else resume()}){
                    Text(if(annotation.saved!=null)"Resume live" else "Discard and resume…")}
            }
        }
    },canvas={
        Box(Modifier.fillMaxSize().onSizeChanged{size=it;input.viewport(it.width.toDouble(),it.height.toDouble(),density)}){
            if(view!=null)key(view){DesktopStreamCanvas(view,Modifier.fillMaxSize())}
            if(adapter!=null&&childState!=null)key(adapter){
                EditorCanvas(adapter.controller,childState,Modifier.fillMaxSize(),acceptInput=false,renderLifetime=adapter.renderLifetime)
            }
            Box(Modifier.fillMaxSize()
                .onPointerEvent(PointerEventType.Scroll){event->
                    if(decision==null&&!locked&&adapter!=null){event.changes.firstOrNull()?.let{change->
                        val p=Point(change.position.x.toDouble(),change.position.y.toDouble())
                        if(event.keyboardModifiers.isCtrlPressed)adapter.controller.zoom(kotlin.math.exp(-change.scrollDelta.y*0.12),p)
                        else adapter.controller.pan(-change.scrollDelta.x*28.0,-change.scrollDelta.y*28.0)
                        change.consume()
                    }}
                }
                .pointerInput(input,density,view){awaitEachGesture{
                    val down=awaitFirstDown(requireUnconsumed=false)
                    val event=currentEvent
                    val acceptedTool=streamContactToolAllowed(down.type,event.buttons.isPrimaryPressed,
                        event.buttons.isSecondaryPressed,event.buttons.isTertiaryPressed)
                    val frame=view?.state?.value?.displayed
                    if(!acceptedTool||frame==null||
                        size.width<=0||size.height<=0||latestState.busy)return@awaitEachGesture
                    fun sample(change:PointerInputChange,shift:Boolean,semantic:Boolean=false)=StreamContactSample(
                        Point(change.position.x.toDouble(),change.position.y.toDouble()),change.uptimeMillis,change.pressure,shift,semantic)
                    val camera=latestAdapter?.state?.value?.view?.camera?:streamContactCamera(frame,size.width.toDouble(),size.height.toDouble())
                    if(!input.down(sample(down,event.keyboardModifiers.isShiftPressed,event.keyboardModifiers.isAltPressed),camera,density))return@awaitEachGesture
                    down.consume()
                    var completed=false
                    try{while(true){
                        val next=awaitPointerEvent()
                        if(next.changes.any{it.id!=down.id&&it.pressed})break
                        val change=next.changes.firstOrNull{it.id==down.id}?:break
                        if(!change.pressed){input.up(sample(change,next.keyboardModifiers.isShiftPressed));change.consume();completed=true;break}
                        input.move(sample(change,next.keyboardModifiers.isShiftPressed));change.consume()
                    }}finally{if(!completed)input.cancel()}
                }})
        }
    })
    childState?.textAnchor?.let{
            var text by remember(adapter,it){mutableStateOf("")}
            AlertDialog(onDismissRequest={adapter?.controller?.clearSelection()},title={Text("Annotation text")},
                text={OutlinedTextField(text,{value->text=value.take(4096)})},
                confirmButton={TextButton(onClick={adapter?.controller?.acceptText(text)}){Text("Add")}},
                dismissButton={TextButton(onClick={adapter?.controller?.clearSelection()}){Text("Cancel")}})
        }
    decision?.let{selected->AlertDialog(onDismissRequest={decision=null},
        title={Text(when(selected){StreamDecision.Close->"Discard draft and close?";StreamDecision.Resume->"Discard draft and resume?";StreamDecision.Degraded->"Save degraded preview?"})},
        text={Text(if(selected==StreamDecision.Degraded)"The exact original is unavailable. Save this preview at its retained resolution with an explicit degraded marker?" else "Unsaved annotations in this temporary draft will be discarded. The paired project remains unchanged.")},
        confirmButton={TextButton(enabled=!state.busy,onClick={decision=null;when(selected){
            StreamDecision.Close->close();StreamDecision.Resume->resume();StreamDecision.Degraded->runAction{session.save(true)}
        }}){Text(if(selected==StreamDecision.Degraded)"Save degraded" else "Discard")}},
        dismissButton={TextButton(onClick={decision=null}){Text("Keep draft")}})}
}
