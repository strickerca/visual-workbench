package com.visualworkbench.android.stream

import org.junit.Test

class StreamPlatformTest {
    @Test fun presentationFreezeLosslessAndRetirementOwnership() { check(runStreamLifecycleContract() == 10) }
    @Test fun existingGestureInputKeepsOnlyAcceptedRealContacts() { check(runStreamInputContract() == 8) }
    @Test fun routerAndNativeSignalBoundariesRetainInputAndPublishClosed() { check(runStreamBoundaryRepairContract() == 11) }
    @Test fun closedReceiverKeepsPixelsAndSettlesEnteredDrawBeforeRetirement() { check(runStreamClosedLifecycleContract() == 2) }
}
