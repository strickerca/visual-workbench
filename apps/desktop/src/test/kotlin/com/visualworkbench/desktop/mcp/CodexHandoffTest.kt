package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlin.coroutines.CoroutineContext
import java.util.ArrayDeque
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.atomic.AtomicInteger

class CodexHandoffTest {
    private class QueuedDispatcher:CoroutineDispatcher(){
        private val tasks=ArrayDeque<Runnable>()
        override fun dispatch(context:CoroutineContext,block:Runnable){tasks.addLast(block)}
        fun drain(){var count=0;while(tasks.isNotEmpty()){check(count++<32);tasks.removeFirst().run()}}
    }
    private fun preview()=CodexPreview("attempt","digest","thread","model","package","manifest","binary","schema","literal","original",emptyList())
    private fun accepted(attempt:String="attempt",thread:String="thread",turn:String="turn",status:String="inProgress")=
        mapOf<String,Any?>("accepted" to true,"attemptId" to attempt,"threadId" to thread,"turnId" to turn,"status" to status)
    @Test fun terminalReceiptBeforeSuspendedAcceptanceResumesCannotBeOverwritten()=runBlocking {
        for(status in listOf("completed","failed","interrupted")){
            val displayed=preview();val state=MutableStateFlow(codexBegin(CodexState(preview=displayed),displayed))
            val dispatcher=QueuedDispatcher();val response=CompletableDeferred<Map<String,Any?>>()
            val send=async(dispatcher,start=CoroutineStart.UNDISPATCHED){codexAccept(state,displayed){response.await()}}
            try{
                // CodexConnection completes its pending reply on the pipe reader;
                // UI resumption remains queued while that reader sees terminal.
                response.complete(accepted());assertFalse(send.isCompleted)
                state.update{codexCompleted(it,"thread","turn",status)}
                val terminal=state.value
                dispatcher.drain();withTimeout(2000){send.await()}
                assertEquals(terminal,state.value);assertTrue(state.value.receipt!!.terminal)
                assertEquals("attempt",state.value.receipt!!.attemptId)
                assertEquals("thread",state.value.receipt!!.threadId)
                assertEquals("turn",state.value.receipt!!.turnId)
                assertEquals(terminal,codexInterrupted(state.value,"attempt"))
            }finally{response.complete(accepted());send.cancel();dispatcher.drain();withContext(NonCancellable){send.join()}}
        }
    }
    @Test fun acceptanceAndCompletionAreExactBoundAndNeverRetireAnotherTurn()=runBlocking {
        val displayed=preview();val state=MutableStateFlow(codexBegin(CodexState(),displayed))
        for(reply in listOf(accepted(attempt="foreign"),accepted(thread="foreign"),accepted(status="unknown"))){
            val before=state.value
            assertTrue(runCatching{codexAccept(state,displayed){reply}}.isFailure)
            assertEquals(before,state.value)
        }
        codexAccept(state,displayed){accepted()};val progress=state.value
        assertEquals(progress,codexCompleted(progress,"foreign","turn","completed"))
        assertEquals(progress,codexCompleted(progress,"thread","foreign","completed"))
        state.update{codexCompleted(it,"thread","turn","completed")};val terminal=state.value
        assertTrue(runCatching{codexAccept(state,displayed){accepted(turn="foreign")}}.isFailure)
        assertEquals(terminal,state.value)
    }
    @Test fun terminalStartReplyAndLatePriorInterruptRemainTerminal()=runBlocking {
        val displayed=preview();val state=MutableStateFlow(codexBegin(CodexState(),displayed))
        codexAccept(state,displayed){accepted(status="completed")};val terminal=state.value
        assertEquals(terminal,codexInterrupted(terminal,"attempt"))
        val next=codexBegin(terminal,displayed.copy(previewId="next"))
        assertEquals(next,codexInterrupted(next,"attempt"))
        assertEquals("next",next.receipt!!.attemptId)
    }
    @Test fun oneOwnerAdmissionRefusesConcurrentRuntimeUntilVerifiedCleanup()=runBlocking {
        val admission=CodexOwnerAdmission();val token=admission.claim();val calls=AtomicInteger()
        assertTrue(runCatching{admission.claim()}.exceptionOrNull() is CodexRefused)
        val owner=CodexSettledOwner(admission,token){calls.incrementAndGet()}
        owner.close();owner.close();assertEquals(1,calls.get());val next=admission.claim();admission.release(next)
    }
    @Test fun concurrentAndCancelledClosersJoinActualProcessSettlement()=runBlocking {
        val admission=CodexOwnerAdmission();val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val calls=AtomicInteger()
        val owner=CodexSettledOwner(admission,admission.claim()){calls.incrementAndGet();entered.complete(Unit);release.await()}
        val first=launch{owner.close()};var second:Job?=null
        try{withTimeout(2000){entered.await()};second=launch{owner.close()};yield();first.cancel();yield()
            assertFalse(first.isCompleted);assertFalse(checkNotNull(second).isCompleted)
            assertTrue(runCatching{admission.claim()}.isFailure)
            release.complete(Unit);withTimeout(2000){first.join();checkNotNull(second).join()};assertEquals(1,calls.get())
            val token=admission.claim();admission.release(token)
        }finally{release.complete(Unit);withContext(NonCancellable){first.cancelAndJoin();second?.cancelAndJoin()}}
    }
    @Test fun uncertainCleanupQuarantinesPinnedResourcesAndRefusesAnotherOwner()=runBlocking {
        val admission=CodexOwnerAdmission();val calls=AtomicInteger();val retained=Any()
        val owner=CodexSettledOwner(admission,admission.claim()){assertNotNull(retained);calls.incrementAndGet();throw CodexRefused("cleanup_uncertain")}
        repeat(2){assertTrue(runCatching{owner.close()}.exceptionOrNull() is CodexRefused)}
        assertEquals(1,calls.get());assertTrue(runCatching{admission.claim()}.exceptionOrNull() is CodexRefused)
    }
    @Test fun stoppedAdmissionCannotBeReleasedUsingAnotherToken(){
        val admission=CodexOwnerAdmission();val token=admission.claim()
        assertTrue(runCatching{admission.release(Any())}.isFailure)
        assertTrue(runCatching{admission.claim()}.isFailure);admission.release(token)
    }
    @Test fun packagedRuntimeMustContainEveryCodexAdapterAndItsMeasuredProfileInput(){
        val required=listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs","vw-codex-host.exe","mcp/codex/main.mjs","mcp/codex/client.mjs","mcp/codex/package.mjs","mcp/codex/process.mjs","mcp/codex/schema.mjs","mcp/codex/schema-worker.mjs","mcp/codex/image-profiles.json")
        fun bytes(names:List<String>)=names.joinToString(""){"${"a".repeat(64)} 1 $it\n"}.toByteArray()
        assertEquals(required.size,DesktopMcpBundle.parse(bytes(required)).size)
        for(missing in required.filter{it.contains("codex")})assertTrue(runCatching{DesktopMcpBundle.parse(bytes(required.filterNot{it==missing}))}.isFailure)
    }
    @Test fun missingMeasuredImageContractProducesActionableRefusal(){
        val text=DesktopCodexHandoff.explain(CodexRefused("unverified_image_preprocessing"))
        assertTrue(text.contains("Send is disabled"));assertTrue(text.contains("MCP pull"));assertTrue(text.contains("no image was sent"))
    }
}
