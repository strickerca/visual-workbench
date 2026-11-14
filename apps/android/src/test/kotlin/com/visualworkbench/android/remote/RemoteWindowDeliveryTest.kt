package com.visualworkbench.android.remote

import org.junit.Assert.*
import org.junit.Test

/** Pure window-delivery witness and provenance mapping used by the integration
 * Activity and captureRemoteInputProvenance. No MotionEvent is constructed. */
class RemoteWindowDeliveryTest {
    private val penDown=RemoteMotionFacts(7,0x5002,false,true,false,0x5002,REMOTE_TOOL_TYPE_STYLUS,0,0,0,0,0,40_000_000L,40L,REMOTE_ACTION_DOWN)
    private val penMove=penDown.copy(actionMasked=2,historySize=1,eventTimeNanos=48_000_000L,eventTimeMillis=48L)

    @Test fun onlyTheEventBeingWindowDispatchedIsDelivered() {
        val ledger=RemoteWindowDeliveryLedger()
        assertEquals(RemoteWindowDelivery(false,RemoteInputVerification.NotDelivered),ledger.delivery(7,40L,REMOTE_ACTION_DOWN))
        val token=ledger.enter(7,40L,REMOTE_ACTION_DOWN,RemoteInputVerification.Verified)
        assertEquals(RemoteWindowDelivery(true,RemoteInputVerification.Verified),ledger.delivery(7,40L,REMOTE_ACTION_DOWN))
        // A different device or event time inside the same dispatch is not this event.
        assertFalse(ledger.delivery(8,40L,REMOTE_ACTION_DOWN).delivered)
        assertFalse(ledger.delivery(7,41L,REMOTE_ACTION_DOWN).delivered)
        ledger.exit(token)
        assertFalse(ledger.delivery(7,40L,REMOTE_ACTION_DOWN).delivered)
        assertEquals(1L,ledger.windowDeliveries)
        assertEquals(0L,ledger.syntheticDispatches)
    }
    @Test fun synthesizedHoverCopiesKeepUnsignedDeliveryAndSignedActionsNeedTheirOwnSignature() {
        val ledger=RemoteWindowDeliveryLedger()
        val token=ledger.enter(7,50L,9,RemoteInputVerification.NotSigned)
        // ViewGroup's synthesized HOVER_ENTER copy keeps device/time; it is unsigned.
        assertEquals(RemoteWindowDelivery(true,RemoteInputVerification.NotSigned),ledger.delivery(7,50L,9))
        // A DOWN observed under a window frame entered for another action is never treated as signed.
        assertEquals(RemoteWindowDelivery(true,RemoteInputVerification.NotSigned),ledger.delivery(7,50L,REMOTE_ACTION_DOWN))
        ledger.exit(token)
    }
    @Test fun nestedFramesRestoreTheOuterDispatch() {
        val ledger=RemoteWindowDeliveryLedger()
        val outer=ledger.enter(7,60L,REMOTE_ACTION_UP,RemoteInputVerification.Refused)
        val inner=ledger.enter(7,61L,2,RemoteInputVerification.NotSigned)
        assertTrue(ledger.delivery(7,61L,2).delivered)
        ledger.exit(inner)
        assertEquals(RemoteWindowDelivery(true,RemoteInputVerification.Refused),ledger.delivery(7,60L,REMOTE_ACTION_UP))
        ledger.exit(outer)
        assertEquals(2L,ledger.windowDeliveries)
    }
    @Test fun syntheticDispatchesAreCountedAndContaminateClassification() {
        val ledger=RemoteWindowDeliveryLedger()
        ledger.syntheticDispatch();ledger.syntheticDispatch()
        assertEquals(2L,ledger.syntheticDispatches)
        val token=ledger.enter(7,48L,2,RemoteInputVerification.NotSigned)
        val provenance=remoteInputProvenance(penMove,ledger.delivery(7,48L,2))
        ledger.exit(token)
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(provenance,ledger.syntheticDispatches))
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(provenance,0L))
    }
    @Test fun provenanceCopiesFactsAndNeverInventsVerification() {
        val delivered=remoteInputProvenance(penDown,RemoteWindowDelivery(true,RemoteInputVerification.Verified))
        assertEquals(RemoteInputProvenance(7,0x5002,false,true,false,0x5002,REMOTE_TOOL_TYPE_STYLUS,0,0,0,0,0,40_000_000L,
            REMOTE_ACTION_DOWN,true,RemoteInputVerification.Verified),delivered)
        assertEquals(RemoteInputOrigin.PhysicalStylus,classifyRemoteInput(delivered,0L))
        // Undelivered events never carry a verification claim.
        val direct=remoteInputProvenance(penDown,RemoteWindowDelivery(false,RemoteInputVerification.Verified))
        assertEquals(RemoteInputVerification.NotDelivered,direct.verification)
        assertEquals(RemoteInputOrigin.Direct,classifyRemoteInput(direct,0L))
        // Instrumentation/UiAutomation-style injection with SOURCE_STYLUS only stays Unknown even when signed.
        val shell=remoteInputProvenance(penDown.copy(source=REMOTE_SOURCE_STYLUS),RemoteWindowDelivery(true,RemoteInputVerification.Verified))
        assertEquals(RemoteInputOrigin.Unknown,classifyRemoteInput(shell,0L))
    }
}
