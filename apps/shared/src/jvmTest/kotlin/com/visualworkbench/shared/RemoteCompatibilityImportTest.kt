package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class RemoteCompatibilityImportTest {
    @Test fun bounded_owner_backed_receipt_import() { assertEquals(10,RemoteCompatibilityImportCases.run()) }
}
