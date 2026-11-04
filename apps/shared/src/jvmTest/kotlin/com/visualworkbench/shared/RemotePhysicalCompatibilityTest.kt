package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class RemotePhysicalCompatibilityTest {
    @Test fun python_physical_rows_decode_and_satisfy_validFor() {
        // Two exact Python-generated tool documents plus nine refused mutations.
        assertEquals(11, RemotePhysicalCompatibilityCases.run())
    }
}
