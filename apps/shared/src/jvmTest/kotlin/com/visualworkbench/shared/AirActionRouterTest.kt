package com.visualworkbench.shared

import org.junit.Test

class AirActionRouterTest {
    @Test fun air_action_contract_scenarios() {
        AirActionContract.scenarios().forEach { (name, scenario) ->
            try { scenario() } catch (failure: Exception) { throw AssertionError(name, failure) }
        }
    }
}
