package com.visualworkbench.desktop.remote

import kotlin.test.Test
import kotlin.test.assertEquals

class RemoteCompatibilityImportTest {
    @Test fun bounded_regular_receipt_files() { assertEquals(5,RemoteCompatibilityFileCases.run()) }
}
