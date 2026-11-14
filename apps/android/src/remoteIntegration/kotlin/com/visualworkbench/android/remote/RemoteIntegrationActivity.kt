package com.visualworkbench.android.remote

import android.graphics.Color
import android.hardware.input.InputManager
import android.os.Bundle
import android.os.SystemClock
import android.view.InputDevice
import android.view.MotionEvent
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
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
    private var prompt:TextView?=null
    private var promptBlock:String?=null
    private var promptStrokes=0
    // Main-thread window-delivery witness. Activity dispatch is entered from this
    // window's input pipeline; DOWN/UP are checked against the dispatcher's
    // signature on entry (API 30+). The synthetic producer below calls the view
    // directly and is counted separately. Observation only, never authority.
    // In-process callers of dispatchTouchEvent fail the DOWN/UP signature check.
    private val ledger=RemoteWindowDeliveryLedger()
    private val inputManager:InputManager? by lazy{getSystemService(InputManager::class.java)}
    val witness:RemoteInputDeliveryWitness=object:RemoteInputDeliveryWitness {
        override fun delivery(event:MotionEvent):RemoteWindowDelivery = ledger.delivery(event.deviceId,event.eventTime,event.actionMasked)
        override val syntheticDispatches:Long get()=ledger.syntheticDispatches
        override val windowDeliveries:Long get()=ledger.windowDeliveries
    }
    private fun enterWindow(event:MotionEvent):Any =
        ledger.enter(event.deviceId,event.eventTime,event.actionMasked,verifyRemoteInputEvent(inputManager,event))
    override fun dispatchTouchEvent(event:MotionEvent):Boolean {
        val prior=enterWindow(event)
        try{return super.dispatchTouchEvent(event)}finally{ledger.exit(prior)}
    }
    override fun dispatchGenericMotionEvent(event:MotionEvent):Boolean {
        val prior=enterWindow(event)
        try{return super.dispatchGenericMotionEvent(event)}finally{ledger.exit(prior)}
    }
    override fun onCreate(state:Bundle?){
        super.onCreate(state);check(packageName=="com.visualworkbench.android.hil")
        val id=requireNotNull(intent.getStringExtra("ownedRunId"))
        val block=intent.getStringExtra("ownerPhysicalBlock")
        val strokes=intent.getIntExtra("ownerPhysicalStrokes",0)
        check(block==null||(block in REMOTE_PHYSICAL_BLOCKS&&strokes in 1..64))
        val remote=requireNotNull(owners.remove(id))
        controller=RemoteEditController(remote,scope)
        view=RemoteSurfaceView(this,controller)
        if(block==null)setContentView(view)
        else{
            // The prompt sits outside the remote surface so it never covers or
            // intercepts the owner's pen on the remote canvas.
            promptBlock=block;promptStrokes=strokes
            val label=TextView(this).apply{
                setTextColor(Color.WHITE);setBackgroundColor(Color.rgb(28,28,30));textSize=16f
                setPadding(40,28,40,28);importantForAccessibility=View.IMPORTANT_FOR_ACCESSIBILITY_YES
                // A fixed line count keeps the prompt height constant. Any change
                // in the remote surface size would seal the active grant.
                setLines(PROMPT_LINES);ellipsize=android.text.TextUtils.TruncateAt.END
                text=ownerPhysicalPrompt(block,strokes,0)
            }
            prompt=label
            setContentView(LinearLayout(this).apply{
                orientation=LinearLayout.VERTICAL
                addView(label,LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,ViewGroup.LayoutParams.WRAP_CONTENT))
                addView(view,LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,0,1f))
            })
        }
        scope.launch{controller.display.collect{view.configure(it.config)}}
    }
    /** Integration-only physical observation. The witness and journal never
     * dispatch, grant, or replay input; only the owner's pen produces samples. */
    internal fun armPhysicalJournal(journal:RemotePhysicalGestureJournal){
        check(promptBlock!=null&&ledger.syntheticDispatches==0L)
        controller.armPhysicalJournal(journal,witness)
    }
    fun ownerProgress(admitted:Int,finished:Boolean=false){
        val block=promptBlock?:return
        prompt?.text=if(finished)"Pen test recorded. You can put the pen down." else ownerPhysicalPrompt(block,promptStrokes,admitted)
    }
    /** Isolated integration variant only: software-generated events enter the
     * actual SurfaceView dispatch route, never native input/ACK methods. */
    fun dispatchOwnedStylusBatch(binding:RemoteTargetBinding,held:Boolean=false,editorStroke:List<Int>?=null):RemoteRenderedFrameObservation {
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
        if(editorStroke!=null){
            check(!held&&editorStroke.size==6&&editorStroke.all{it in 1000..999000})
            check(editorStroke[2]-editorStroke[0]==1000&&editorStroke[4]-editorStroke[0]==2000)
            check(editorStroke[1]==editorStroke[3]&&editorStroke[1]==editorStroke[5])
        }
        val downTime=SystemClock.uptimeMillis()
        try {
            val actions=if(held)listOf(MotionEvent.ACTION_DOWN,MotionEvent.ACTION_MOVE)else listOf(MotionEvent.ACTION_DOWN,MotionEvent.ACTION_MOVE,MotionEvent.ACTION_UP)
            for((index,action)in actions.withIndex()){
                check(lifecycle.currentState==Lifecycle.State.RESUMED&&hasWindowFocus()&&controller.remote.target.value?.let{it.grantActive&&it.binding==binding}==true)
                val properties=MotionEvent.PointerProperties().apply{id=0;toolType=MotionEvent.TOOL_TYPE_STYLUS}
                val point=MotionEvent.PointerCoords().apply{
                    if(editorStroke==null){
                        x=viewport.left+viewport.width/2f+index*2f;y=viewport.top+viewport.height/2f
                    }else{
                        x=viewport.left+viewport.width*editorStroke[index*2]/1_000_000f
                        y=viewport.top+viewport.height*editorStroke[index*2+1]/1_000_000f
                    }
                    pressure=if(action==MotionEvent.ACTION_UP)0f else 0.5f;size=1f
                    setAxisValue(MotionEvent.AXIS_TILT,0f);setAxisValue(MotionEvent.AXIS_ORIENTATION,0f)
                }
                val event=MotionEvent.obtain(downTime,SystemClock.uptimeMillis(),action,1,arrayOf(properties),arrayOf(point),0,0,1f,1f,0,0,InputDevice.SOURCE_STYLUS,0)
                ledger.syntheticDispatch()
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
        private const val PROMPT_LINES=6
        fun ownerPhysicalPrompt(block:String,strokes:Int,admitted:Int):String {
            val paint=block.startsWith("paint-")
            val where=if(paint)"the Paint canvas" else "the test window"
            val task=when(block.removePrefix("paint-")){
                "pressure"->"Draw $strokes strokes on $where. Alternate: light to firm in one stroke, firm to light in the next."
                "hover"->"Hold the pen just above $where and sweep across it $strokes times. Don't touch the screen."
                "barrel"->"Hold the pen's side button and draw $strokes strokes on $where."
                "eraser"->"Turn the pen to its eraser end and draw $strokes strokes on $where. A pen without an eraser end can't finish this step."
                "eraser-tool"->"With the Eraser tool selected in Paint, erase across the earlier strokes $strokes times."
                else->"Draw $strokes strokes on $where."
            }
            return listOf("Pen test: $task","Use only the pen. A finger or palm touch stops the test.","Recorded: $admitted of $strokes").joinToString("\n")
        }
        private val owners=ConcurrentHashMap<String,WorkbenchRemoteEdit>()
        private val pending=ConcurrentHashMap.newKeySet<RemoteEditController>()
        private val retirement=CoroutineScope(SupervisorJob()+Dispatchers.Default)
        fun bind(id:String,remote:WorkbenchRemoteEdit){require(id.matches(Regex("[a-f0-9]{32}")));check(owners.putIfAbsent(id,remote)==null)}
        fun unbind(id:String){owners.remove(id)}
        fun pendingOwners():Int=pending.size
    }
}
