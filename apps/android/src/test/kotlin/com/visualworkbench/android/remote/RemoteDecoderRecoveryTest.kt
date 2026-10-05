package com.visualworkbench.android.remote

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test

class RemoteDecoderRecoveryTest {
    @Test fun missingCallbackRetiresBeforeRequestingReplacement() = runBlocking {
        val events=mutableListOf<String>();var calls=0
        retireRemoteDecoderRecovery(
            retire={events+="retire";if(++calls<3)RemoteRecoveryRetirement.Pending else RemoteRecoveryRetirement.Complete},
            request={events+="request"},pending={events+="pending"},
        )
        assertEquals(listOf("retire","pending","retire","pending","retire","request"),events)
    }
    @Test fun staleRecoveryCannotRequestFramesForReplacementOwner() = runBlocking {
        var calls=0;var requests=0
        retireRemoteDecoderRecovery(
            retire={if(++calls==1)RemoteRecoveryRetirement.Pending else RemoteRecoveryRetirement.Obsolete},
            request={requests++},pending={},
        )
        assertEquals(2,calls);assertEquals(0,requests)
    }
    @Test fun retirementCancellationDoesNotInventCompletionOrKeyframe() = runBlocking {
        var requests=0
        try {
            retireRemoteDecoderRecovery(retire={RemoteRecoveryRetirement.Pending},request={requests++},
                pending={throw CancellationException("owner retirement takes over")})
            fail("Cancellation lost")
        } catch (_:CancellationException) { }
        assertEquals(0,requests)
    }
}
