package com.visualworkbench.desktop.remote

import org.junit.Test
import org.junit.Assert.assertEquals

class RemoteCompatibilityImportTest {
    @Test fun bounded_regular_receipt_files() { assertEquals(5,RemoteCompatibilityFileCases.run()) }
}
