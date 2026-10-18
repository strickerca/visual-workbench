package com.visualworkbench.android.airactions

import org.junit.Test

class AirActionRemoteMotionTest {
    @Test fun remote_motion_router_and_delegate_contract() {
        AirActionRemoteMotionContract.scenarios().forEach { (name, scenario) ->
            try { scenario() } catch (failure: Exception) { throw AssertionError(name, failure) }
        }
    }
}
