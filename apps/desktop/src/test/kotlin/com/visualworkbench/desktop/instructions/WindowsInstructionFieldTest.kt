package com.visualworkbench.desktop.instructions

import javax.swing.SwingUtilities
import org.junit.Test
import org.junit.Assert.*

class WindowsInstructionFieldTest {
    @Test fun programmaticRefreshDoesNotMasqueradeAsTypedInput() = onEdt {
        val view = WindowsInstructionField(); var calls = 0
        try { view.changed = { calls++; true }; view.apply("retained", true, "field"); assertEquals(0, calls)
            view.field.append(" typed"); assertEquals(1, calls); assertEquals("retained typed", view.field.text)
        } finally { view.release() }
    }
    @Test fun overLimitAndNulEditsPreserveThePriorLiteralValue() = onEdt {
        val view = WindowsInstructionField()
        try { view.apply("prior", true, "field"); view.field.append("界".repeat(12000)); assertEquals("prior", view.field.text)
            view.field.append("\u0000"); assertEquals("prior", view.field.text)
        } finally { view.release() }
    }
    @Test fun disposedTextFieldDoesNotDeliverAQueuedInputCallback() = onEdt {
        val view = WindowsInstructionField(); var calls = 0
        view.changed = { calls++; true }; view.release(); view.field.append("late"); assertEquals(0, calls)
    }
    @Test fun platformInputMethodsHaveARealEditableSwingDestination() = onEdt {
        val view = WindowsInstructionField()
        try { view.apply("voice destination", true, "field"); assertTrue(view.field.isEditable)
            assertEquals("Instruction text", view.field.accessibleContext.accessibleName)
            view.apply("busy", false, "field"); assertFalse(view.field.isEditable)
        } finally { view.release() }
    }
    private fun onEdt(block: () -> Unit) { if (SwingUtilities.isEventDispatchThread()) block() else SwingUtilities.invokeAndWait(block) }
}
