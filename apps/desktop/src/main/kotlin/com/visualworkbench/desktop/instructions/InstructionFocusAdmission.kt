package com.visualworkbench.desktop.instructions

/** Owner EDT only. Swing FileDialog runs a nested event loop, so this admission
 * starts before construction/showing and survives until the picker finally exits. */
internal class InstructionFocusAdmission(private val changed: () -> Unit = {}) {
    var modal: Boolean = false
        set(value) { if (field != value) { field = value; changed() } }
    private var pickers = 0
    fun picker(): AutoCloseable {
        check(pickers < 8) { "Too many nested file pickers" }
        pickers++; changed()
        var closed = false
        return AutoCloseable { if (!closed) { closed = true; pickers--; changed() } }
    }
    fun allows(textDraft: Boolean, export: Boolean, paste: Boolean): Boolean =
        !modal && pickers == 0 && !textDraft && !export && !paste
}
