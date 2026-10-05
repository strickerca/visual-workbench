package com.visualworkbench.android.remote

import android.os.Bundle
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import androidx.lifecycle.Lifecycle
import com.visualworkbench.shared.RemoteTargetBinding
import com.visualworkbench.shared.RemoteViewport
import androidx.activity.ComponentActivity
import com.visualworkbench.shared.WorkbenchRemoteEdit
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.*

/** Included only in the explicit isolated integration source set. The fixture
 * binds a real native remote owner before launch; no Intent can supply authority. */
internal class RemoteIntegrationActivity:ComponentActivity() {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
    lateinit var controller:RemoteEditController;private set
    private lateinit var view:RemoteSurfaceView
    override fun onCreate(state:Bundle?){
        super.onCreate(state);check(packageName=="com.visualworkbench.android.hil")
        val id=requireNotNull(intent.getStringExtra("ownedRunId"))
        val remote=requireNotNull(owners.remove(id))
        controller=RemoteEditController(remote,scope)
        view=RemoteSurfaceView(this,controller);setContentView(view)
        scope.launch{controller.display.collect{view.configure(it.config)}}
    }
    /** Isolated integration variant only: software-generated events enter the
     * actual SurfaceView dispatch route, never native input/ACK methods. */
    fun dispatchOwnedStylusBatch(binding:RemoteTargetBinding,held:Boolean=false):RemoteRenderedFrameObservation {
        check(lifecycle.currentState==Lifecycle.State.RESUMED&&hasWindowFocus())
        val target=requireNotNull(controller.remote.target.value)
        check(target.grantActive&&target.binding==binding)
        val current=requireNotNull(controller.renderedObservation.value)
        val display=controller.display.value;val config=requireNotNull(display.config)
        check(current.scope==config.scope&&current.scope.connectionEpoch==binding.connectionEpoch&&
            current.scope.captureSessionId==binding.captureSessionId&&current.scope.sourceGeneration==binding.sourceGeneration&&
            current.scope.targetToken==binding.targetToken&&current.scope.geometryRevision==binding.geometryRevision)
        val viewport=requireNotNull(RemoteViewport.fit(display.viewWidth,display.viewHeight,config.visibleWidth,config.visibleHeight))
        check(view.width==display.viewWidth&&view.height==display.viewHeight&&viewport.width>=32f&&viewport.height>=32f)
        val downTime=SystemClock.uptimeMillis()
        try {
            val actions=if(held)listOf(MotionEvent.ACTION_DOWN,MotionEvent.ACTION_MOVE)else listOf(MotionEvent.ACTION_DOWN,MotionEvent.ACTION_MOVE,MotionEvent.ACTION_UP)
            for((index,action)in actions.withIndex()){
                check(lifecycle.currentState==Lifecycle.State.RESUMED&&hasWindowFocus()&&controller.remote.target.value?.let{it.grantActive&&it.binding==binding}==true)
                val properties=MotionEvent.PointerProperties().apply{id=0;toolType=MotionEvent.TOOL_TYPE_STYLUS}
                val point=MotionEvent.PointerCoords().apply{
                    x=viewport.left+viewport.width/2f+index*2f;y=viewport.top+viewport.height/2f
                    pressure=if(action==MotionEvent.ACTION_UP)0f else 0.5f;size=1f
                    setAxisValue(MotionEvent.AXIS_TILT,0f);setAxisValue(MotionEvent.AXIS_ORIENTATION,0f)
                }
                val event=MotionEvent.obtain(downTime,SystemClock.uptimeMillis(),action,1,arrayOf(properties),arrayOf(point),0,0,1f,1f,0,0,InputDevice.SOURCE_STYLUS,0)
                try{check(view.dispatchTouchEvent(event))}finally{event.recycle()}
            }
        }catch(error:Throwable){controller.deactivate("owner_pause");throw error}
        return current
    }
    override fun onResume(){super.onResume();if(::controller.isInitialized)controller.foreground()}
    override fun onPause(){
        if(::controller.isInitialized){controller.deactivate("background");scope.launch{controller.background()}}
        super.onPause()
    }
    override fun onDestroy(){
        if(::controller.isInitialized){
            controller.invalidate();pending.add(controller)
            // Parent cancellation never releases a pending native/Surface owner.
            retirement.launch {while(!controller.retire())delay(20);pending.remove(controller);scope.cancel()}
        }else scope.cancel()
        super.onDestroy()
    }
    companion object {
        private val owners=ConcurrentHashMap<String,WorkbenchRemoteEdit>()
        private val pending=ConcurrentHashMap.newKeySet<RemoteEditController>()
        private val retirement=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        fun bind(id:String,remote:WorkbenchRemoteEdit){require(id.matches(Regex("[a-f0-9]{32}")));check(owners.putIfAbsent(id,remote)==null)}
        fun unbind(id:String){owners.remove(id)}
        fun pendingOwners():Int=pending.size
    }
}
