package com.visualworkbench.android.instructions

internal enum class InstructionFocusModal { ExportDialog, ExportPicker }

/** Owner UI dispatcher only. The two external modal lifetimes overlap while an
 * export dialog hands off to a document picker; dismissing one cannot unseal the other. */
internal class InstructionFocusAdmission(private val changed: () -> Unit = {}) {
    private val owners = mutableSetOf<InstructionFocusModal>()
    private val leases = mutableSetOf<Any>()
    /** Independent picker leases survive a previous composition clearing its
     * Boolean modal flags. Only the exact owner token can release this fence. */
    fun hold(): AutoCloseable {
        val token = Any(); leases.add(token); changed()
        return AutoCloseable { if (leases.remove(token)) changed() }
    }
    fun modal(owner: InstructionFocusModal, active: Boolean) {
        val updated = if (active) owners.add(owner) else owners.remove(owner)
        if (updated) changed()
    }
    fun clear() { if (owners.isNotEmpty()) { owners.clear(); changed() } }
    fun allows(textDraft: Boolean, brush: Boolean, contextMenu: Boolean, blocked: Boolean): Boolean =
        owners.isEmpty() && leases.isEmpty() && !textDraft && !brush && !contextMenu && !blocked
}
