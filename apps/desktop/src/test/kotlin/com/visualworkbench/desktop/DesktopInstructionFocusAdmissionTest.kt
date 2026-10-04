package com.visualworkbench.desktop

import com.visualworkbench.desktop.instructions.InstructionFocusAdmission
import org.junit.Assert.*
import org.junit.Test

class DesktopInstructionFocusAdmissionTest {
    @Test fun nestedNativePickerAndComposeModalLifetimesCannotReleaseEachOther() {
        val gate = InstructionFocusAdmission()
        val first = gate.picker(); val second = gate.picker()
        try {
            first.close(); first.close()
            assertFalse(gate.allows(false, false, false))
            gate.modal = true; second.close()
            assertFalse(gate.allows(false, false, false))
            gate.modal = false; assertTrue(gate.allows(false, false, false))
        } finally { first.close(); second.close() }
    }
    @Test fun synchronousAdmissionPrecedesNestedEventLoopAndFinallyRestoresAfterFailure() {
        var changes = 0; val gate = InstructionFocusAdmission { changes++ }
        try {
            gate.picker().use {
                assertFalse(gate.allows(false, false, false))
                throw IllegalStateException("injected picker failure")
            }
        } catch (_: IllegalStateException) { }
        assertTrue(gate.allows(false, false, false)); assertEquals(2, changes)
        assertFalse(gate.allows(true, false, false))
        assertFalse(gate.allows(false, true, false))
        assertFalse(gate.allows(false, false, true))
    }
}
