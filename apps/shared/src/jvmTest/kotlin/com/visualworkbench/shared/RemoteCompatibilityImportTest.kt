package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals

class RemoteCompatibilityImportTest {
    @Test fun bounded_owner_backed_receipt_import() {
        // The runner also checks borrow closure and joined observation retirement
        // on success, failure and cancellation, alongside admission and palette use.
        assertEquals(16, RemoteCompatibilityImportCases.run())
    }
}
