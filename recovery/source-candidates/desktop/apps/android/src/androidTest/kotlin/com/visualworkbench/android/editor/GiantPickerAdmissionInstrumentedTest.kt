package com.visualworkbench.android.editor

import android.net.Uri
import com.visualworkbench.android.instructions.InstructionFocusAdmission
import com.visualworkbench.android.instructions.InstructionFocusModal
import org.junit.Assert.*
import org.junit.Test

class GiantPickerAdmissionInstrumentedTest {
    private fun allowed(value: InstructionFocusAdmission) = value.allows(false, false, false, false)

    @Test fun delayedResultAfterEditorReentryRemainsGuardedThroughConfirmation() {
        val admission = InstructionFocusAdmission()
        var pending = false; var selected: Uri? = null; var imported: Uri? = null
        val owner = GiantPickerAdmission(false, admission::hold) { wait, uri -> pending = wait; selected = uri }
        try {
            owner.launch { assertFalse(allowed(admission)) }
            assertTrue(pending); assertFalse(allowed(admission))
            // The actual editor may now replace Projects with an open canvas.
            admission.clear(); assertFalse(allowed(admission))
            owner.result(Uri.parse("content://synthetic/giant.jpg"))
            assertFalse(pending); assertNotNull(selected); assertFalse(allowed(admission))
            owner.accept { assertFalse(allowed(admission)); imported = it }
            assertNotNull(imported); assertNull(selected); assertTrue(allowed(admission))
        } finally { owner.close() }
    }
    @Test fun recreatedPickerOwnsIndependentLeaseBeforeOldCompositionDisposes() {
        val admission = InstructionFocusAdmission()
        var saved = false; var shown: Uri? = null
        val old = GiantPickerAdmission(false, admission::hold) { pending, _ -> saved = pending }
        old.launch { }
        val replacement = GiantPickerAdmission(saved, admission::hold) { _, uri -> shown = uri }
        try {
            old.close(); admission.clear(); assertFalse(allowed(admission))
            old.result(Uri.parse("content://synthetic/old")); assertNull(shown)
            replacement.result(Uri.parse("content://synthetic/current"))
            assertEquals("content://synthetic/current", shown.toString()); assertFalse(allowed(admission))
            replacement.dismiss(); assertTrue(allowed(admission))
        } finally { old.close(); replacement.close() }
    }
    @Test fun failedLaunchNullResultAndDisposalReleaseOnlyTheirOwnLease() {
        val admission = InstructionFocusAdmission()
        admission.modal(InstructionFocusModal.ExportDialog, true)
        val owner = GiantPickerAdmission(false, admission::hold) { _, _ -> }
        try {
            try { owner.launch { throw IllegalStateException("controlled picker refusal") }; fail("failure hidden") }
            catch (_: IllegalStateException) { }
            assertFalse(allowed(admission)) // An unrelated export modal remains.
            admission.modal(InstructionFocusModal.ExportDialog, false); assertTrue(allowed(admission))
            owner.launch { }; owner.result(null); assertTrue(allowed(admission))
            owner.launch { }; owner.close(); assertTrue(allowed(admission))
        } finally { owner.close() }
    }
    @Test fun unsolicitedLateResultAndSecondLaunchCannotReplaceAnOwnedConfirmation() {
        val admission = InstructionFocusAdmission(); var selected: Uri? = null; var launches = 0
        val owner = GiantPickerAdmission(false, admission::hold) { _, uri -> selected = uri }
        try {
            owner.result(Uri.parse("content://synthetic/unsolicited")); assertNull(selected); assertTrue(allowed(admission))
            owner.launch { launches++ }; owner.launch { launches++ }
            owner.result(Uri.parse("content://synthetic/first"))
            owner.result(Uri.parse("content://synthetic/second")); owner.launch { launches++ }
            assertEquals(1, launches); assertEquals("content://synthetic/first", selected.toString())
        } finally { owner.close() }
    }
}
