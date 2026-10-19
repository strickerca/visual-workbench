package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow

/** Owned synthetic receipt fixtures exercise admission, never product evidence. */
internal object RemoteCompatibilityImportCases {
    fun run():Int = runBlocking {
        val target=target()
        val bytes=receipt()
        val decoded=NativeRemoteCompatibility.decode(target,bytes)
        val owner=FixtureOwner(target,bytes,decoded.records)
        check(NativeRemoteCompatibility.admitCurrent(target,bytes,{target},owner)!=null)
        check(owner.enters==1&&owner.exits==1)
        val unknown=FixtureOwner(target,bytes,emptyList())
        check(NativeRemoteCompatibility.admit(target,bytes,unknown)==null)
        check(NativeRemoteCompatibility.admit(target,bytes,object:NativeCompatibilityObservationAccess {
            override suspend fun withCurrentReceipt(expected:RemoteTargetState,receiptSha256:String,
                body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?):RemoteCompatibilityAdmission? = body(null)
        })==null)
        check(NativeRemoteCompatibility.admit(target,bytes,FixtureOwner(target,receipt("Unsupported"),decoded.records))==null)
        check(NativeRemoteCompatibility.admit(target,bytes,FixtureOwner(target.copy(identity=target.identity.copy(fileVersion="11.2605.82.0")),bytes,decoded.records))==null)
        val invalid=listOf(
            byteArrayOf(0xc3.toByte(),0x28),ByteArray(32769){32},byteArrayOf(),
            receipt().toString(Charsets.UTF_8).replace("\"schema\":1","\"schema\":1,\"schema\":1").toByteArray(),
            receipt().toString(Charsets.UTF_8).replace("\"schema\":1","\"schema\":1,\"trusted\":true").toByteArray(),
            receipt().toString(Charsets.UTF_8).replace("Drawing","Pressure").toByteArray(),
            receipt().toString(Charsets.UTF_8).replace("EditorObserved","PhysicalOwnerGesture").replace("Mouse","WindowsPointerInput").replace("Drawing","Pressure").toByteArray(),
            receipt().toString(Charsets.UTF_8).replace("Mouse","Unknown").toByteArray(),
        )
        invalid.forEach{raw->check(runCatching{NativeRemoteCompatibility.decode(target,raw)}.isFailure)}
        var current:RemoteTargetState?=target
        val stale=object:NativeCompatibilityObservationAccess {
            override suspend fun withCurrentReceipt(expected:RemoteTargetState,receiptSha256:String,
                body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?):RemoteCompatibilityAdmission? {
                val result=body(NativeOwnedCompatibilityReceipt(target,"brush","c".repeat(64),bytes,decoded.records))
                current=target.copy(binding=target.binding.copy(connectionEpoch=2uL))
                return result
            }
        }
        check(NativeRemoteCompatibility.admitCurrent(target,bytes,{current},stale)==null)
        val borrows=NativeCompatibilityBorrowOwner()
        val entered=CompletableDeferred<Unit>()
        val retirement=CompletableDeferred<Unit>()
        var borrowerRetired=false
        val borrower=launch(start=CoroutineStart.UNDISPATCHED) {
            try {borrows.borrow {
                entered.complete(Unit)
                try {awaitCancellation()} finally {withContext(NonCancellable){retirement.await()};borrowerRetired=true}
            }} catch(_:CancellationException) {}
        }
        entered.await()
        var closeFinished=false
        val closeFirst=launch(start=CoroutineStart.UNDISPATCHED) {borrows.closeAndJoin();closeFinished=true}
        check(!closeFinished&&!borrowerRetired)
        retirement.complete(Unit)
        closeFirst.join()
        check(closeFinished&&borrowerRetired&&borrower.isCompleted)
        check(borrows.borrow { 1 }==null)
        var released=0
        val token=Any()
        check(withJoinedCompatibilityObservation({token},{check(it===token);released++}) {
            check(it===token&&released==0);42
        }==42&&released==1)
        check(runCatching { withJoinedCompatibilityObservation({token},{released++}) {
            throw IllegalArgumentException("callback fixture")
        }}.isFailure&&released==2)
        check(runCatching { withJoinedCompatibilityObservation<Any,Unit>(
            {throw IllegalArgumentException("producer fixture")},{released++},{})
        }.isFailure&&released==2)
        val producerEntered=CompletableDeferred<Unit>()
        val producerReturn=java.util.concurrent.CountDownLatch(1)
        val producerReleased=java.util.concurrent.atomic.AtomicInteger()
        var callbackEntered=false
        val undelivered=launch(Dispatchers.Default) {
            withJoinedCompatibilityObservation({
                producerEntered.complete(Unit)
                check(producerReturn.await(5,java.util.concurrent.TimeUnit.SECONDS));token
            },{producerReleased.incrementAndGet()}) {callbackEntered=true}
        }
        producerEntered.await();undelivered.cancel();producerReturn.countDown();undelivered.join()
        check(!callbackEntered&&producerReleased.get()==1)
        val callbackReady=CompletableDeferred<Unit>()
        val cancelledRead=launch(start=CoroutineStart.UNDISPATCHED) {
            withJoinedCompatibilityObservation({token},{released++}) {callbackReady.complete(Unit);awaitCancellation()}
        }
        callbackReady.await();check(released==2);cancelledRead.cancelAndJoin();check(released==3)
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val dispatcher=FixtureDispatcher(target,owner)
        val palette=RemoteEditorPalette(dispatcher,scope)
        try {
            check(palette.importCompatibilityReceipt(bytes,target))
            check(palette.state.value.compatibility==decoded.records)
            check(palette.state.value.target!!.verifiedActions==setOf(RemoteEditorAction.Undo))
            check(dispatcher.commands==0)
            check(palette.observeCompatibility())
            check(palette.state.value.compatibility.all{it.result==RemoteCompatibilityResult.NotTested})
            check(palette.state.value.compatibility.size==7&&dispatcher.commands==0)
            check(palette.state.value.target!!.verifiedActions==setOf(RemoteEditorAction.Undo))
            check(palette.state.value.message!!.contains("Not tested"))
            check(palette.importCompatibilityReceipt(bytes,target))
            val unowned=receipt("Unsupported")
            check(palette.importCompatibilityReceipt(unowned,target))
            check(palette.state.value.compatibility==decoded.records)
            check(palette.state.value.message!!.contains("not admitted"))
            owner.gate=CompletableDeferred();owner.retire=CompletableDeferred()
            check(palette.importCompatibilityReceipt(bytes,target))
            check(!palette.command(RemoteEditorAction.Undo))
            var closed=false
            val closing=scope.launch{palette.close();closed=true}
            check(!closed&&owner.cancelled)
            check(palette.state.value.busy)
            owner.retire!!.complete(Unit)
            check(closed&&closing.isCompleted&&!palette.state.value.busy)
            check(owner.enters==owner.exits)
        } finally { owner.retire?.complete(Unit);palette.close();scope.cancel() }
        16
    }
    private fun target():RemoteTargetState {
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,"mspaint.exe","a".repeat(64),1024uL,"11.2605.81.0",
            "Microsoft.Paint_11.2605.81.0_x64__publisher","11.2605.81.0","b".repeat(64))
        return RemoteTargetState(binding,identity,setOf(RemoteEditorAction.Undo),
            RemoteCanvasFocusProof("UIA","d".repeat(64),42u,"target",1u,100uL,"canvas",50026,RemotePhysicalRect(0,0,10u,10u),"b".repeat(64)),true,null)
    }
    private fun receipt(result:String="Supported"):ByteArray = """{"schema":1,"kind":"paint-compatibility-observations","identity":{"editor":"Paint","executableName":"mspaint.exe","executableBlake3":"${"a".repeat(64)}","executableBytes":1024,"fileVersion":"11.2605.81.0","packageFullName":"Microsoft.Paint_11.2605.81.0_x64__publisher","packageVersion":"11.2605.81.0","shortcutProfileDigest":"${"b".repeat(64)}"},"toolId":"brush","settingsDigest":"${"c".repeat(64)}","observations":[{"aspect":"Drawing","result":"$result","method":"EditorObserved","inputApi":"Mouse","evidenceSha256":"${"e".repeat(64)}","deviceModel":null}]}""".toByteArray()
    private class FixtureOwner(val target:RemoteTargetState,val bytes:ByteArray,val records:List<RemoteToolCompatibility>):NativeCompatibilityObservationAccess {
        var enters=0;var exits=0;var cancelled=false
        var gate:CompletableDeferred<Unit>?=null;var retire:CompletableDeferred<Unit>?=null
        override suspend fun withCurrentReceipt(expected:RemoteTargetState,receiptSha256:String,
            body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?):RemoteCompatibilityAdmission? {
            enters++
            try {
                try{gate?.await()}catch(error:CancellationException){cancelled=true;withContext(NonCancellable){retire?.await()};throw error}
                if(receiptSha256!=NativeRemoteCompatibility.sha(bytes))return body(null)
                return body(NativeOwnedCompatibilityReceipt(target,"brush","c".repeat(64),bytes,records))
            }finally{exits++}
        }
    }
    private class FixtureDispatcher(initial:RemoteTargetState,val owner:NativeCompatibilityObservationAccess):RemoteCommandDispatcher,RemoteCompatibilityImportCapability {
        override val target=MutableStateFlow<RemoteTargetState?>(initial)
        var commands=0
        override suspend fun dispatch(binding:RemoteTargetBinding,action:RemoteEditorAction):RemoteCommandReceipt {
            commands++;return RemoteCommandReceipt(binding,action,1uL,1uL,RemoteCommandStatus.Injected,null)
        }
        override suspend fun admitCompatibilityReceipt(expected:RemoteTargetState,bytes:ByteArray):RemoteCompatibilityAdmission? =
            NativeRemoteCompatibility.admitCurrent(expected,bytes,{target.value},owner)
        override suspend fun observeCompatibility(expected:RemoteTargetState):RemoteCompatibilityAdmission? =
            RemoteCompatibilityAdmission(expected,"paint.brush","c".repeat(64),RemoteCompatibilityAspect.entries.map {
                RemoteToolCompatibility(expected.identity,"paint.brush","c".repeat(64),RemoteEditorInputApi.Unknown,
                    it,RemoteCompatibilityResult.NotTested,RemoteCompatibilityMethod.NotPerformed,null,null)
            })
    }
}
public fun main():Unit {println("Compatibility import synthetic groups passed: ${RemoteCompatibilityImportCases.run()}")}
