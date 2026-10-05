package com.visualworkbench.android.remote

import com.visualworkbench.shared.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test

class RemoteUiCommandsTest {
    private fun state(status:RemoteEditStatus,selected:Boolean=true)=RemoteEditState(1uL,status,
        if(selected)RemoteVideoScope(1uL,"019f7c21-5678-7123-8123-0123456789ab",1uL,"a".repeat(32),1u)else null,null,null,null,null)
    @Test fun openBeforePcSelectionCannotDispatchRequest()=runBlocking {
        var calls=0;val before=state(RemoteEditStatus.Selecting,false)
        assertFalse(remoteUiCommandEnabled(RemoteUiCommand.RequestControl,before))
        assertNotNull(runRemoteUiCommand(RemoteUiCommand.RequestControl,{before},{false}){calls++})
        assertEquals(0,calls)
    }
    @Test fun disconnectBetweenClickAndCallIsAVisibleTypedRefusal()=runBlocking {
        val selected=state(RemoteEditStatus.Viewing);assertTrue(remoteUiCommandEnabled(RemoteUiCommand.RequestControl,selected))
        val message=runRemoteUiCommand(RemoteUiCommand.RequestControl,{selected},{false}){throw SessionFailure(SessionFailureKind.Transport)}
        assertEquals("The link changed; reconnect and reselect the PC window.",message)
    }
    @Test fun latestClosedOrUnselectedStateRefusesWithoutCallingNative()=runBlocking {
        for(next in listOf(state(RemoteEditStatus.Disconnected),state(RemoteEditStatus.Closed),state(RemoteEditStatus.Viewing,false))) {
            var called=false;assertNotNull(runRemoteUiCommand(RemoteUiCommand.RequestControl,{next},{false}){called=true});assertFalse(called)
        }
    }
    @Test fun pauseRefusalAndPendingAreCaughtWithoutInventingRetirement()=runBlocking {
        val active=state(RemoteEditStatus.Controlling)
        assertTrue(remoteUiCommandEnabled(RemoteUiCommand.Pause,active))
        assertEquals("Retirement is pending; keep this view open.",runRemoteUiCommand(RemoteUiCommand.Pause,{active},{false}){throw SessionFailure(SessionFailureKind.RemoteRetirementPending)})
        assertNotNull(runRemoteUiCommand(RemoteUiCommand.Pause,{active},{false}){throw SessionFailure(SessionFailureKind.Invalid)})
    }
    @Test fun cancellationIsPreservedAndNotReportedAsARefusal()=runBlocking {
        val original=CancellationException("owned command cancelled")
        try{runRemoteUiCommand(RemoteUiCommand.Pause,{state(RemoteEditStatus.Controlling)},{false}){throw original};fail("Cancellation swallowed")}
        catch(caught:CancellationException){assertSame(original,caught)}
    }
    @Test fun closingBlocksAnAlreadyAdmittedActionAndUnknownErrorsLeakNoMessage()=runBlocking {
        var called=false;assertNotNull(runRemoteUiCommand(RemoteUiCommand.Pause,{state(RemoteEditStatus.Controlling)},{true}){called=true});assertFalse(called)
        assertEquals("The command failed; no retry was sent.",runRemoteUiCommand(RemoteUiCommand.RequestControl,{state(RemoteEditStatus.Viewing)},{false}){throw IllegalStateException("private endpoint")})
    }
    @Test fun aSuccessfulRequestReturnsBeforeUiInputIsResumed()=runBlocking {
        var completed=false;assertNull(runRemoteUiCommand(RemoteUiCommand.RequestControl,{state(RemoteEditStatus.Viewing)},{false}){completed=true});assertTrue(completed)
        assertFalse(remoteUiCommandEnabled(RemoteUiCommand.RequestControl,state(RemoteEditStatus.Controlling)))
    }
}
