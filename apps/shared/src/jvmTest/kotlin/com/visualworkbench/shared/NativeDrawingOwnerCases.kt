package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** Synthetic foreign producer/session boundary; the actual consumer registry,
 * owner and production preparation function are compiled by the focused probe. */
internal object NativeDrawingOwnerCases {
    internal fun target():RemoteTargetState {
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,"mspaint.exe","a".repeat(64),1024uL,"11.2605.81.0",
            "Microsoft.Paint_11.2605.81.0_x64__publisher","11.2605.81.0","b".repeat(64))
        return RemoteTargetState(binding,identity,emptySet(),null,true,null)
    }
    private fun receipt(target:RemoteTargetState):NativeOwnedCompatibilityReceipt {
        val image=target.identity
        val rows=RemoteCompatibilityAspect.entries.joinToString(",") {aspect->
            if(aspect==RemoteCompatibilityAspect.Drawing)
                """{"aspect":"Drawing","result":"Supported","method":"EditorObserved","inputApi":"Mouse","evidenceSha256":"${"e".repeat(64)}","deviceModel":null}"""
            else """{"aspect":"$aspect","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null}"""
        }
        val bytes="""{"schema":1,"kind":"paint-compatibility-observations","identity":{"editor":"Paint","executableName":"${image.executableName}","executableBlake3":"${image.executableBlake3}","executableBytes":${image.executableBytes},"fileVersion":"${image.fileVersion}","packageFullName":"${image.packageFullName}","packageVersion":"${image.packageVersion}","shortcutProfileDigest":"${image.shortcutProfileDigest}"},"toolId":"paint.brush","settingsDigest":"${"c".repeat(64)}","observations":[$rows]}""".toByteArray()
        val value=NativeRemoteCompatibility.decode(target,bytes)
        return NativeOwnedCompatibilityReceipt(target,value.toolId,value.settingsDigest,bytes,value.records)
    }
    private class Prepared(override var target:RemoteTargetState):NativeDrawingPreparation {
        var runs=0;var closes=0;var cancellations=0;var failures=0;var destroyed=false
        var runGate:CompletableDeferred<Unit>?=null
        var closeGate:CompletableDeferred<Unit>?=null
        var cancelGate:CountDownLatch?=null
        val cancelEntered=CompletableDeferred<Unit>()
        val closeEntered=CompletableDeferred<Unit>()
        var result:NativeOwnedCompatibilityReceipt?=receipt(target)
        override fun cancel(){
            check(!destroyed);cancellations++;cancelEntered.complete(Unit)
            cancelGate?.let{check(it.await(3,TimeUnit.SECONDS))}
            check(!destroyed)
        }
        override suspend fun runConfirmed():NativeOwnedCompatibilityReceipt? {
            check(!destroyed);runs++;runGate?.await();return result
        }
        override suspend fun retire():RemoteDrawingCanvasState {
            check(!destroyed);closes++;closeEntered.complete(Unit);closeGate?.await()
            if(failures>0){failures--;error("actual retained owner fixture")}
            destroyed=true
            return if(runs==0)RemoteDrawingCanvasState.Unchanged else RemoteDrawingCanvasState.Restored
        }
    }
    private class Fixture {
        val expected=target();val prepared=Prepared(expected)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        var prepares=0;var prepareGate:CompletableDeferred<Unit>?=null
        val adapter:NativeDrawingAdapterFixture
        init {
            val access=object:NativeDrawingPreparationAccess {
                override suspend fun prepare(expected:RemoteTargetState):NativeDrawingPreparation {
                    check(adapter.owners().size==1);check(expected==this@Fixture.expected)
                    prepares++;prepareGate?.await();return prepared
                }
            }
            adapter=NativeDrawingAdapterFixture(expected,scope,access)
        }
        suspend fun close(){prepareGate?.complete(Unit);prepared.runGate?.complete(Unit);prepared.closeGate?.complete(Unit);prepared.cancelGate?.countDown();adapter.closeSession();scope.cancel()}
    }
    private suspend fun settled(predicate:()->Boolean){withTimeout(3_000){while(!predicate())yield()}}
    fun run():Int=runBlocking {
        var passed=0
        suspend fun case(name:String,body:suspend(Fixture)->Unit){
            val f=Fixture();try{withTimeout(4_000){body(f)};passed++;println("PASS native drawing owner: $name")}
            finally{f.close()}
        }
        case("registry precedes allocation and retains escaped preparation after delivery") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected)
            check(f.prepares==1&&f.prepared.runs==0&&f.adapter.owners().single()===owned)
            check(owned.close()==RemoteDrawingCanvasState.Unchanged)
            check(f.prepared.destroyed&&f.adapter.owners().isEmpty())
        }
        case("one confirmation invokes one actual producer and closed mouse-only receipt") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected)
            val admitted=owned.runConfirmed();check(admitted?.validFor(f.expected)==true)
            check(runCatching{owned.runConfirmed()}.isFailure&&f.prepared.runs==1)
            check(owned.close()==RemoteDrawingCanvasState.Restored);check(f.adapter.owners().isEmpty())
            check(owned.close()==RemoteDrawingCanvasState.Restored&&f.prepared.closes==1)
        }
        case("session close retires preparation still waiting for confirmation") {f->
            f.adapter.prepareDrawingCheck(f.expected);f.adapter.closeSession()
            check(f.prepared.runs==0&&f.prepared.destroyed&&f.adapter.parentDestroyed.get())
        }
        case("late foreign allocation remains registered until cancelled preparation retires") {f->
            f.prepareGate=CompletableDeferred()
            val pending=f.scope.async(start=CoroutineStart.UNDISPATCHED){f.adapter.prepareDrawingCheck(f.expected)}
            check(f.prepares==1);val closing=f.scope.async(start=CoroutineStart.UNDISPATCHED){f.adapter.closeSession()}
            check(!closing.isCompleted&&!f.adapter.parentDestroyed.get()&&f.adapter.owners().size==1)
            f.prepareGate!!.complete(Unit)
            check(runCatching{pending.await()}.isFailure);closing.await()
            check(f.prepared.destroyed&&f.prepared.runs==0&&f.adapter.parentDestroyed.get())
        }
        case("failed foreign cleanup retains same session owner and parent handle for retry") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected);f.prepared.failures=1
            check(runCatching{f.adapter.closeSession()}.isFailure)
            check(!f.adapter.parentDestroyed.get()&&!f.prepared.destroyed&&f.adapter.owners().single()===owned)
            f.adapter.closeSession();check(f.prepared.destroyed&&f.prepared.closes==2&&f.adapter.parentDestroyed.get())
        }
        case("concurrent UI and session close share one foreign retirement") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected);f.prepared.closeGate=CompletableDeferred()
            val ui=f.scope.async(start=CoroutineStart.UNDISPATCHED){owned.close()};f.prepared.closeEntered.await()
            val session=f.scope.async(start=CoroutineStart.UNDISPATCHED){f.adapter.closeSession()}
            check(!ui.isCompleted&&!session.isCompleted&&f.prepared.closes==1)
            f.prepared.closeGate!!.complete(Unit);ui.await();session.await()
            check(f.prepared.closes==1&&f.adapter.parentDestroyed.get())
        }
        case("foreign cancellation already in flight completes before handle destruction") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected);f.prepared.cancelGate=CountDownLatch(1)
            val invalidating=f.scope.launch(Dispatchers.Default){f.adapter.replace(null)}
            f.prepared.cancelEntered.await()
            val closing=f.scope.async(start=CoroutineStart.UNDISPATCHED){owned.close()}
            check(!closing.isCompleted&&f.prepared.closes==0&&!f.prepared.destroyed)
            f.prepared.cancelGate!!.countDown();invalidating.join();closing.await()
            check(f.prepared.closes==1&&f.prepared.cancellations==1&&f.prepared.destroyed)
        }
        case("session cancellation joins actual running producer before destroying its owner") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected);f.prepared.runGate=CompletableDeferred()
            val running=f.scope.async(start=CoroutineStart.UNDISPATCHED){owned.runConfirmed()}
            settled{f.prepared.runs==1};f.adapter.closeSession()
            check(runCatching{running.await()}.isFailure)
            check(f.prepared.destroyed&&f.prepared.cancellations>0&&f.adapter.parentDestroyed.get())
        }
        case("source change before confirmation cannot call producer") {f->
            val owned=f.adapter.prepareDrawingCheck(f.expected);f.adapter.replace(null)
            check(runCatching{owned.runConfirmed()}.isFailure&&f.prepared.runs==0)
            owned.close();check(f.prepared.destroyed)
        }
        case("foreign target metadata failure disposes allocated owner before returning") {f->
            f.prepared.target=f.expected.copy(binding=f.expected.binding.copy(connectionEpoch=3uL))
            check(runCatching{f.adapter.prepareDrawingCheck(f.expected)}.isFailure)
            check(f.prepared.destroyed&&f.adapter.owners().isEmpty()&&f.prepared.runs==0)
        }
        case("closed registry refuses preparation before actual allocation") {f->
            f.adapter.closeSession();check(runCatching{f.adapter.prepareDrawingCheck(f.expected)}.isFailure)
            check(f.prepares==0&&f.adapter.owners().isEmpty())
        }
        case("positive decoder refuses missing aspects and any physical or shortcut inference") {f->
            val good=receipt(f.expected)
            fun rejects(bytes:ByteArray){
                val decoded=NativeRemoteCompatibility.decode(f.expected,bytes)
                val owned=NativeOwnedCompatibilityReceipt(f.expected,decoded.toolId,decoded.settingsDigest,bytes,decoded.records)
                check(runCatching{NativeRemoteCompatibility.admitDrawing(f.expected,owned)}.isFailure)
            }
            val text=good.receipt.toString(Charsets.UTF_8)
            rejects(text.replace("\"inputApi\":\"Mouse\"","\"inputApi\":\"WindowsPointerInput\"").toByteArray())
            rejects(text.replace("paint.brush","paint.pencil").toByteArray())
            val one=text.substringBefore(",{\"aspect\":\"Pressure\"")+"]}"
            rejects(one.toByteArray())
            check(runCatching{NativeRemoteCompatibility.admitDrawing(f.expected,
                NativeOwnedCompatibilityReceipt(f.expected.copy(grantActive=false),good.toolId,good.settingsDigest,good.receipt,good.records))}.isFailure)
            check(runCatching{NativeRemoteCompatibility.admitDrawing(f.expected,
                NativeOwnedCompatibilityReceipt(f.expected,good.toolId,good.settingsDigest,good.receipt,good.records.dropLast(1)))}.isFailure)
        }
        passed
    }
}
/** Replaces only the parent/foreign boundary; registration, late allocation,
 * task cancellation and retirement use the actual production implementation. */
internal class NativeDrawingAdapterFixture(
    expected:RemoteTargetState,private val scope:CoroutineScope,private val access:NativeDrawingPreparationAccess,
):RemoteDrawingCheckCapability {
    private val consumers=RemoteConsumerRegistry()
    private val closing=AtomicBoolean()
    private val target=MutableStateFlow<RemoteTargetState?>(expected)
    val parentDestroyed=AtomicBoolean()
    override val drawingCheckSupported:Boolean get()=!closing.get()
    override suspend fun prepareDrawingCheck(expected:RemoteTargetState):RemotePreparedDrawingCheck = prepareNativeDrawing(
        expected,access,scope,consumers,current={
            !closing.get()&&target.value?.let{it.binding==expected.binding&&it.identity==expected.identity&&it.grantActive}==true
        },checkOpen={if(closing.get())throw SessionFailure(SessionFailureKind.Closed)},
    )
    fun owners():List<RemoteConsumerOwner> = consumers.snapshot()
    fun replace(value:RemoteTargetState?){target.value=value;consumers.invalidate()}
    suspend fun closeSession(){
        if(parentDestroyed.get())return
        closing.set(true);consumers.invalidate()
        for(consumer in consumers.fence()){
            if(!consumer.retire())throw SessionFailure(SessionFailureKind.RemoteRetirementPending)
            consumers.retired(consumer)
        }
        parentDestroyed.set(true)
    }
}
public fun main():Unit {println("Native drawing preparation cases passed: ${NativeDrawingOwnerCases.run()}")}
