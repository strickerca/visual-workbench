package com.visualworkbench.shared

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class RemoteConsumerOwnershipTest {
    private class Consumer:RemoteConsumerOwner{var invalidations=0;override fun invalidate(){invalidations++};override suspend fun retire()=true}
    @Test fun malformed_config_discards_exact_ticket_without_render_ack()=runBlocking {
        val discarded=mutableListOf<ULong>()
        try{ownRemoteTicket(7uL,{discarded+=it}){RemoteNativeJson.config("{}");Unit};fail("Malformed config accepted")}
        catch(_:IllegalStateException){}
        assertEquals(listOf(7uL),discarded)
    }
    @Test fun cancelled_delivery_waits_for_actual_discard_before_owner_finishes()=runBlocking {
        val entered=CompletableDeferred<Unit>();val discarded=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val operation=launch{ownRemoteTicket(8uL,{assertEquals(8uL,it);discarded.complete(Unit);release.await()}){entered.complete(Unit);awaitCancellation()}}
        try{withTimeout(3000){entered.await()};operation.cancel();withTimeout(3000){discarded.await()};assertFalse(operation.isCompleted);release.complete(Unit);withTimeout(3000){operation.join()}}
        finally{release.complete(Unit);operation.cancelAndJoin()}
    }
    @Test fun close_fence_and_racing_registration_never_lose_an_admitted_consumer()=runBlocking {
        repeat(100){
            val registry=RemoteConsumerRegistry();val owner=Consumer();val start=CompletableDeferred<Unit>()
            val register=async(Dispatchers.Default){start.await();try{registry.register(owner);true}catch(_:SessionFailure){false}}
            val close=async(Dispatchers.Default){start.await();registry.fence()}
            start.complete(Unit);val accepted=register.await();val fenced=close.await()
            assertEquals(accepted,fenced.contains(owner));assertEquals(accepted,registry.snapshot().contains(owner))
            try{registry.register(Consumer());fail("Admission after close fence")}catch(_:SessionFailure){}
        }
    }
    @Test fun a_pending_consumer_remains_in_retry_snapshot_after_other_owner_retires(){
        val registry=RemoteConsumerRegistry();val pending=Consumer();val done=Consumer()
        registry.register(pending);registry.register(done);registry.fence();registry.retired(done)
        assertEquals(listOf(pending),registry.fence());registry.invalidate();assertEquals(1,pending.invalidations)
    }

    @Test fun consumed_malformed_receipt_seals_and_awaits_retirement_without_cancel_ticket()=runBlocking {
        var paused=false;var cancelCalled=false;var observations=0
        val original=IllegalStateException("Malformed native status")
        try{ownRemoteCommandAttempt<Unit>({throw original},{retireIncompleteRemoteCommand(true,
            pause={paused=true},cancel={cancelCalled=true;error("consumed ticket")},retired={observations++;paused})});fail("Malformed receipt accepted")}
        catch(error:IllegalStateException){assertSame(original,error)}
        assertTrue(paused);assertFalse(cancelCalled);assertEquals(1,observations)
    }
    @Test fun missing_cancel_ticket_still_observes_retirement_and_preserves_primary_failure()=runBlocking {
        var paused=false;var observed=false;val primary=IllegalArgumentException("Wrong binding");val missing=IllegalStateException("Missing ticket")
        try{ownRemoteCommandAttempt<Unit>({throw primary},{retireIncompleteRemoteCommand(false,
            pause={paused=true},cancel={throw missing},retired={observed=true;assertTrue(paused);true})});fail("Wrong binding accepted")}
        catch(error:IllegalArgumentException){assertSame(primary,error);assertEquals(listOf(missing),error.suppressed.toList())}
        assertTrue(observed)
    }
    @Test fun cancellation_waits_for_real_retirement_and_keeps_original_cancellation()=runBlocking {
        val started=CompletableDeferred<Unit>();val observing=CompletableDeferred<Unit>();val released=CompletableDeferred<Unit>();val finished=CompletableDeferred<Throwable>()
        val actualPrimary=CompletableDeferred<Throwable>()
        val cancellation=CancellationException("owner cancelled")
        val operation=launch{try{ownRemoteCommandAttempt<Unit>({started.complete(Unit);try{awaitCancellation()}catch(error:Throwable){actualPrimary.complete(error);throw error}},{retireIncompleteRemoteCommand(true,
            pause={},cancel={error("taken ticket")},retired={observing.complete(Unit);released.await();true})})}catch(error:Throwable){finished.complete(error);throw error}}
        try{
            withTimeout(3000){started.await()};operation.cancel(cancellation);withTimeout(3000){observing.await()};assertFalse(operation.isCompleted)
            released.complete(Unit);withTimeout(3000){operation.join()}
            val primary=withTimeout(3000){actualPrimary.await()}
            assertSame(primary,withTimeout(3000){finished.await()})
            // Stack-trace recovery may copy the caller's cancellation, but
            // the actual captured primary must survive and retain its cause.
            assertTrue("Caller cancellation identity was lost",generateSequence(primary){it.cause}.take(16).any{it===cancellation})
        }finally{released.complete(Unit);operation.cancelAndJoin()}
    }
    @Test fun cancellation_during_retirement_does_not_replace_captured_protocol_failure()=runBlocking {
        val observing=CompletableDeferred<Unit>();val released=CompletableDeferred<Unit>();val finished=CompletableDeferred<Throwable>()
        val primary=IllegalStateException("Malformed receipt before cancellation")
        var actualRetired=false
        val operation=launch{try{ownRemoteCommandAttempt<Unit>({throw primary},{retireIncompleteRemoteCommand(true,
            pause={},cancel={error("taken ticket")},retired={observing.complete(Unit);released.await();actualRetired=true;true})})}catch(error:Throwable){finished.complete(error)}}
        try{
            withTimeout(3000){observing.await()};operation.cancel(CancellationException("Cancelled during actual retirement"));assertFalse(operation.isCompleted)
            released.complete(Unit);withTimeout(3000){operation.join()}
            assertTrue(actualRetired);assertSame(primary,withTimeout(3000){finished.await()})
        }finally{released.complete(Unit);operation.cancelAndJoin()}
    }
    @Test fun unknown_retirement_remains_pending_and_does_not_replace_dto_failure()=runBlocking {
        val primary=IllegalStateException("Unknown DTO status");var paused=false;var queried=false
        try{ownRemoteCommandAttempt<Unit>({throw primary},{retireIncompleteRemoteCommand(true,
            pause={paused=true},cancel={},retired={queried=true;error("native query unavailable")})});fail("Bad status accepted")}
        catch(error:IllegalStateException){assertSame(primary,error);assertEquals(1,error.suppressed.size);assertEquals(SessionFailureKind.RemoteRetirementPending,(error.suppressed.single() as SessionFailure).kind)}
        assertTrue(paused);assertTrue(queried)
    }

    @Test fun command_native_cleanup_runs_on_worker_instead_of_calling_dispatcher()=runBlocking {
        val caller=Thread.currentThread();var cleanup:Thread?=null
        ownRemoteCommandAttempt({Unit},{cleanup=Thread.currentThread()})
        assertNotNull(cleanup);assertNotSame(caller,cleanup)
    }

    @Test fun command_acceptance_fields_are_checked_without_silently_zeroing_bad_dtos(){
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        val action=RemoteEditorAction.Undo
        val injected=checkedRemoteCommandReceipt(binding,action,"injected",7uL,101uL,null)
        assertEquals(7uL,injected.inputSequence);assertEquals(101uL,injected.acceptedQpc100ns)
        for(status in listOf("refused","sealed_partial")){
            val receipt=checkedRemoteCommandReceipt(binding,action,status,0uL,0uL,"native_refused")
            assertEquals(0uL,receipt.inputSequence);assertEquals(0uL,receipt.acceptedQpc100ns)
            for(fields in listOf(7uL to 0uL,0uL to 101uL,7uL to 101uL)){
                try{checkedRemoteCommandReceipt(binding,action,status,fields.first,fields.second,"native_refused");fail("Non-injected acceptance fields admitted")}
                catch(_:IllegalArgumentException){}
            }
        }
        for(fields in listOf(0uL to 101uL,7uL to 0uL,0uL to 0uL)){
            try{checkedRemoteCommandReceipt(binding,action,"injected",fields.first,fields.second,null);fail("Zero injected acceptance admitted")}
            catch(_:IllegalArgumentException){}
        }
    }
    @Test fun malformed_numeric_taken_receipt_preserves_error_and_seals_retirement()=runBlocking {
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        var paused=false;var retired=false
        try{ownRemoteCommandAttempt({checkedRemoteCommandReceipt(binding,RemoteEditorAction.Undo,"refused",7uL,101uL,"native_refused")},
            {retireIncompleteRemoteCommand(true,pause={paused=true},cancel={error("Already consumed")},retired={retired=true;paused})});fail("Malformed acceptance admitted")}
        catch(error:IllegalArgumentException){assertTrue(error.suppressed.isEmpty())}
        assertTrue(paused);assertTrue(retired)
    }
}
