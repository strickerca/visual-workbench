package com.visualworkbench.android.airactions

import org.junit.Test

class AirActionWindowKeyTest {
    @Test fun public_window_key_dispatch_contract() {
        AirActionWindowKeyContract.scenarios().forEach { (name, scenario) ->
            try { scenario() } catch (failure: Exception) { throw AssertionError(name, failure) }
        }
    }
}
