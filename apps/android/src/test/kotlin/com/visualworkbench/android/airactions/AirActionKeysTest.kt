package com.visualworkbench.android.airactions

import org.junit.Test

class AirActionKeysTest {
    @Test fun declared_key_contract_scenarios() {
        AirActionKeyContract.scenarios().forEach { (name, scenario) ->
            try { scenario() } catch (failure: Exception) { throw AssertionError(name, failure) }
        }
    }
}
