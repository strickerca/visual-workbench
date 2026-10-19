package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex

/** Synthetic opaque-owner lifecycle fixtures; no product drawing evidence. */
internal object RemoteDrawingCheckCases {
    private fun target():RemoteTargetState {
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,"mspaint.exe","a".repeat(64),1024uL,"11.2605.81.0",
            "Microsoft.Paint_11.2605.81.0_x64__publisher","11.2605.81.0","b".repeat(64))
        return RemoteTargetState(binding,identity,emptySet(),null,true,null)
    }
    private class Prepared(override var target:RemoteTargetState):RemotePreparedDrawingCheck {
        var runs=0;var closes=0;var failures=0
        var runGate:CompletableDeferred<Unit>?=null
        var closeGate:CompletableDeferred<Unit>?=null
        val closeEntered=CompletableDeferred<Unit>()
        var canvas:RemoteDrawingCanvasState?=null
        var resultTarget:RemoteTargetState?=null
        override suspend fun runConfirmed():RemoteCompatibilityAdmission {
            runs++;runGate?.await()
            val source=resultTarget?:target
            return RemoteCompatibilityAdmission(source,"paint.brush","c".repeat(64),
                listOf(RemoteToolCompatibility(source.identity,"paint.brush","c".repeat(64),RemoteEditorInputApi.Mouse,
                    RemoteCompatibilityAspect.Drawing,RemoteCompatibilityResult.Supported,
                    RemoteCompatibilityMethod.EditorObserved,"e".repeat(64),null)))
        }
        override suspend fun close():RemoteDrawingCanvasState {
            closes++;closeEntered.complete(Unit);closeGate?.await()
            if(failures>0){failures--;error("owned cleanup fixture")}
            return canvas?:if(runs==0)RemoteDrawingCanvasState.Unchanged else RemoteDrawingCanvasState.Restored
        }
    }
    private class Fixture {
        val expected=target()
        val prepared=Prepared(expected)
        val source=MutableStateFlow<RemoteTargetState?>(expected)
        val parent=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val admission=Mutex()
        var prepares=0;var adopted=0;var supported=true;var accept=true
        var prepareGate:CompletableDeferred<Unit>?=null
        val backend:RemoteCommandDispatcher=object:RemoteCommandDispatcher,RemoteDrawingCheckCapability {
            override val target=source
            override val drawingCheckSupported:Boolean get()=supported
            override suspend fun dispatch(binding:RemoteTargetBinding,action:RemoteEditorAction):RemoteCommandReceipt =
                error("Drawing flow must not use ordinary command dispatch")
            override suspend fun prepareDrawingCheck(expected:RemoteTargetState):RemotePreparedDrawingCheck {
                check(expected==source.value);prepares++;prepareGate?.await();return prepared
            }
        }
        val flow=RemoteDrawingCheck(backend,parent,admission,adopt={
            check(prepared.closes>0);check(it.target==expected);if(accept)adopted++;accept
        })
        fun unlocked(){check(admission.tryLock());admission.unlock()}
        suspend fun close(){prepareGate?.complete(Unit);prepared.runGate?.complete(Unit);prepared.closeGate?.complete(Unit);flow.close();parent.cancel()}
    }
    private suspend fun settled(test:()->Boolean){withTimeout(3_000){while(!test())yield()}}
    fun run():Int=runBlocking {
        var passed=0
        suspend fun case(name:String,body:suspend(Fixture)->Unit){
            val f=Fixture();try{withTimeout(4_000){body(f)};passed++;println("PASS drawing flow: $name")}
            finally{f.close()}
        }
        case("query-only preparation holds command admission until confirmation") {f->
            check(f.flow.prepare());check(f.prepares==1&&f.prepared.runs==0&&f.adopted==0)
            check(f.flow.state.value.phase==RemoteDrawingCheckPhase.AwaitingConfirmation)
            check(!f.admission.tryLock());check(!f.flow.prepare())
            check(f.flow.confirm());settled{f.flow.state.value.phase==RemoteDrawingCheckPhase.Complete}
            check(f.prepared.runs==1&&f.prepared.closes==1&&f.adopted==1);f.unlocked()
        }
        case("cancel before confirmation sends no input") {f->
            check(f.flow.prepare());f.flow.cancel();settled{f.prepared.closes==1}
            check(f.prepared.runs==0&&f.adopted==0);f.unlocked()
        }
        case("source replacement retires prepared owner without confirmation") {f->
            check(f.flow.prepare());f.source.value=f.expected.copy(binding=f.expected.binding.copy(geometryRevision=2u))
            settled{f.prepared.closes==1};check(!f.flow.confirm()&&f.prepared.runs==0&&f.adopted==0);f.unlocked()
        }
        case("late cancelled preparation remains owned until disposal") {f->
            f.prepareGate=CompletableDeferred();check(f.flow.prepare());f.flow.cancel()
            check(f.prepared.closes==0&&!f.admission.tryLock())
            f.prepareGate!!.complete(Unit);settled{f.prepared.closes==1}
            check(f.prepared.runs==0&&f.adopted==0);f.unlocked()
        }
        case("cleanup failure retains same owner and command permit") {f->
            f.prepared.failures=1;check(f.flow.prepare());check(f.flow.confirm())
            settled{f.flow.state.value.cleanupPending};check(f.prepared.closes==1&&f.adopted==0)
            check(!f.flow.prepare()&&!f.admission.tryLock());f.flow.retryCleanup()
            check(f.prepared.closes==2&&f.prepared.runs==1&&f.adopted==1&&!f.flow.state.value.cleanupPending);f.unlocked()
        }
        case("cancelled run reports native canvas recovery separately from safe retirement") {f->
            f.prepared.runGate=CompletableDeferred();f.prepared.canvas=RemoteDrawingCanvasState.RecoveryNeeded
            check(f.flow.prepare());check(f.flow.confirm());check(f.prepared.runs==1)
            f.flow.cancel();settled{f.prepared.closes==1}
            check(f.flow.state.value.canvasRecoveryNeeded&&!f.flow.state.value.cleanupPending&&f.adopted==0);f.unlocked()
        }
        case("canvas recovery fence survives missing observations and same-session geometry refresh") {f->
            f.prepared.runGate=CompletableDeferred();f.prepared.canvas=RemoteDrawingCanvasState.RecoveryNeeded
            check(f.flow.prepare());check(f.flow.confirm());f.flow.cancel();settled{f.prepared.closes==1}
            check(!f.flow.prepare());f.source.value=null
            f.source.value=f.expected.copy(binding=f.expected.binding.copy(geometryRevision=2u))
            check(!f.flow.prepare()&&f.prepares==1);f.unlocked()
            val successor=f.expected.copy(binding=f.expected.binding.copy(inputSessionId="new-input"))
            f.prepared.target=successor;f.source.value=successor
            check(f.flow.prepare()&&f.prepares==2);f.flow.cancel();settled{f.prepared.closes==2};f.unlocked()
        }
        case("concurrent close waits for same actual retirement") {f->
            f.prepared.closeGate=CompletableDeferred();check(f.flow.prepare());check(f.flow.confirm())
            f.prepared.closeEntered.await()
            val first=async(start=CoroutineStart.UNDISPATCHED){f.flow.close()}
            val second=async(start=CoroutineStart.UNDISPATCHED){f.flow.close()}
            check(!first.isCompleted&&!second.isCompleted&&f.prepared.closes==1)
            f.prepared.closeGate!!.complete(Unit);first.await();second.await()
            check(f.prepared.closes==1&&f.adopted==0&&f.flow.state.value.phase==RemoteDrawingCheckPhase.Closed);f.unlocked()
        }
        case("foreign preparation target is never executed") {f->
            f.prepared.target=f.expected.copy(binding=f.expected.binding.copy(connectionEpoch=2uL))
            check(f.flow.prepare());settled{f.prepared.closes==1}
            check(f.prepared.runs==0&&f.adopted==0);f.unlocked()
        }
        case("foreign result does not become compatibility") {f->
            f.prepared.resultTarget=f.expected.copy(binding=f.expected.binding.copy(connectionEpoch=2uL))
            check(f.flow.prepare());check(f.flow.confirm());settled{f.prepared.closes==1}
            check(f.adopted==0&&f.flow.state.value.phase==RemoteDrawingCheckPhase.Complete);f.unlocked()
        }
        case("rejected adoption is not a cleanup failure") {f->
            f.accept=false;check(f.flow.prepare());check(f.flow.confirm());settled{f.prepared.closes==1}
            check(f.adopted==0&&!f.flow.state.value.cleanupPending);f.unlocked()
        }
        case("unavailable native runtime cannot acquire preparation") {f->
            f.supported=false;check(!f.flow.prepare()&&f.prepares==0);f.unlocked()
        }
        case("closed flow cannot touch native gateway") {f->
            f.flow.close();check(!f.flow.prepare()&&!f.flow.confirm()&&f.prepares==0);f.unlocked()
        }
        passed
    }
}
public fun main():Unit {println("Drawing check lifecycle cases passed: ${RemoteDrawingCheckCases.run()}")}
