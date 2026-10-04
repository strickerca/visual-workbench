package com.visualworkbench.android.instructions

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class InstructionFocusAdmissionInstrumentedTest {
    @Test fun actualTextBrushContextAndBlockingStatesEachRefusePeerAdmission() {
        val gate = InstructionFocusAdmission()
        assertTrue(gate.allows(false, false, false, false))
        assertFalse(gate.allows(true, false, false, false))
        assertFalse(gate.allows(false, true, false, false))
        assertFalse(gate.allows(false, false, true, false))
        assertFalse(gate.allows(false, false, false, true))
    }
    @Test fun exportPickerHandoffStaysSealedWhenDialogDismissesAndReleaseIsIdempotent() {
        var changes = 0; val gate = InstructionFocusAdmission { changes++ }
        gate.modal(InstructionFocusModal.ExportDialog, true)
        gate.modal(InstructionFocusModal.ExportPicker, true)
        gate.modal(InstructionFocusModal.ExportDialog, false)
        assertFalse(gate.allows(false, false, false, false))
        gate.modal(InstructionFocusModal.ExportPicker, false)
        assertTrue(gate.allows(false, false, false, false))
        gate.modal(InstructionFocusModal.ExportPicker, false)
        assertEquals(4, changes)
    }
}
