package com.visualworkbench.android.remote

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class RemoteUiInputAdmissionTest {
    private val scope=RemoteVideoScope(1uL,"019f7c21-5678-7123-8123-0123456789ab",1uL,"a".repeat(32),1u)
    private fun state(value:RemoteVideoScope=scope,status:RemoteEditStatus=RemoteEditStatus.Viewing)=RemoteEditState(1uL,status,value,null,null,null,null)
    @Test fun requestSuccessAfterBackgroundNeverInvokesTheActualResumeCallback()=runBlocking {
        val admission=RemoteUiInputAdmission();val ticket=admission.capture(scope);val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        var resumed=true;var callbacks=0
        val request=async(start=CoroutineStart.UNDISPATCHED){runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{state()},{false},{resumed},{entered.complete(Unit);release.await()},{callbacks++})}
        try{entered.await();resumed=false;admission.revoke();release.complete(Unit);assertNotNull(request.await());assertEquals(0,callbacks)}
        finally{release.complete(Unit);request.cancelAndJoin()}
    }
    @Test fun backgroundThenResumeCannotReviveAnOldRequestToken()=runBlocking {
        val admission=RemoteUiInputAdmission();val ticket=admission.capture(scope);val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();var callbacks=0
        val request=async(start=CoroutineStart.UNDISPATCHED){runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{state()},{false},{true},{entered.complete(Unit);release.await()},{callbacks++})}
        try{entered.await();admission.revoke();admission.revoke();release.complete(Unit);assertNotNull(request.await());assertEquals(0,callbacks)}
        finally{release.complete(Unit);request.cancelAndJoin()}
    }
    @Test fun changedFullScopeAndClosingRefuseSuccessfulRequestResume()=runBlocking {
        for(next in listOf(scope.copy(connectionEpoch=2uL),scope.copy(captureSessionId="019f7c21-5678-7123-8123-0123456789ac"),scope.copy(targetToken="b".repeat(32)),scope.copy(geometryRevision=2u))) {
            val admission=RemoteUiInputAdmission();val ticket=admission.capture(scope);var latest=state();var callbacks=0
            assertNotNull(runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{latest},{false},{true},{latest=state(next)},{callbacks++}));assertEquals(0,callbacks)
        }
        val admission=RemoteUiInputAdmission();val ticket=admission.capture(scope);var closing=false;var callbacks=0
        assertNotNull(runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{state()},{closing},{true},{closing=true},{callbacks++}));assertEquals(0,callbacks)
    }
    @Test fun freshResumedSameScopeRequestRunsTheResumeCallbackExactlyOnce()=runBlocking {
        val admission=RemoteUiInputAdmission();val ticket=admission.capture(scope);var latest=state();var callbacks=0
        assertNull(runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{latest},{false},{true},{latest=state(status=RemoteEditStatus.PendingGrant)},{callbacks++}));assertEquals(1,callbacks)
    }
    @Test fun cancelledOrRefusedRequestsCannotResumeAndCancellationIsPreserved()=runBlocking {
        val admission=RemoteUiInputAdmission();val ticket=admission.capture(scope);var callbacks=0;val cancellation=CancellationException("owned request cancelled")
        try{runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{state()},{false},{true},{throw cancellation},{callbacks++});fail("Cancellation swallowed")}
        catch(caught:CancellationException){assertSame(cancellation,caught)}
        assertNotNull(runRemoteUiCommandWithAdmission(RemoteUiCommand.RequestControl,admission,ticket,{state()},{false},{true},{throw SessionFailure(SessionFailureKind.Transport)},{callbacks++}));assertEquals(0,callbacks)
    }
    @Test fun pauseDoesNotResumeEvenAfterSuccessfulDelivery()=runBlocking {
        val admission=RemoteUiInputAdmission();var callbacks=0
        assertNull(runRemoteUiCommandWithAdmission(RemoteUiCommand.Pause,admission,null,{state(status=RemoteEditStatus.Controlling)},{false},{true},{},{callbacks++}));assertEquals(0,callbacks)
    }
}
