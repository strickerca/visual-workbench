package com.visualworkbench.desktop.instructions

import com.visualworkbench.shared.instructionTextFits
import java.awt.Color
import java.awt.Font
import javax.swing.JTextArea
import javax.swing.JScrollPane
import javax.swing.event.DocumentEvent
import javax.swing.event.DocumentListener
import javax.swing.text.AbstractDocument
import javax.swing.text.AttributeSet
import javax.swing.text.DocumentFilter

/** Ordinary Windows text-services destination, not key injection or a speech
 * recognizer. The owner invokes Win+H; actual OS/provider behavior stays visible. */
internal class WindowsInstructionField : JScrollPane() {
    val field = JTextArea()
    var changed: (String) -> Boolean = { true }
    var focused: () -> Unit = {}
    private var applying = false
    private var focusKey: String? = null
    init {
        field.lineWrap = true; field.wrapStyleWord = true
        field.font = Font(Font.SANS_SERIF, Font.PLAIN, 15)
        field.background = Color(24, 34, 46); field.foreground = Color(238, 243, 248)
        field.caretColor = field.foreground
        field.accessibleContext.accessibleName = "Instruction text"
        field.enableInputMethods(true)
        field.addFocusListener(object : java.awt.event.FocusAdapter() {
            override fun focusGained(event: java.awt.event.FocusEvent) { focused() }
        })
        viewport.view = field
        (field.document as AbstractDocument).documentFilter = object : DocumentFilter() {
            override fun insertString(fb: FilterBypass, offset: Int, string: String?, attrs: AttributeSet?) = replace(fb, offset, 0, string, attrs)
            override fun replace(fb: FilterBypass, offset: Int, length: Int, text: String?, attrs: AttributeSet?) {
                val addition = text.orEmpty()
                if (fb.document.length - length + addition.length > 32768) return
                val before = fb.document.getText(0, fb.document.length)
                if (instructionTextFits(before.substring(0, offset) + addition + before.substring(offset + length))) super.replace(fb, offset, length, addition, attrs)
            }
        }
        field.document.addDocumentListener(object : DocumentListener {
            private fun change() { if (!applying) changed(field.text) }
            override fun insertUpdate(e: DocumentEvent) = change()
            override fun removeUpdate(e: DocumentEvent) = change()
            override fun changedUpdate(e: DocumentEvent) = change()
        })
    }
    fun apply(text: String, editable: Boolean, session: String) {
        field.isEditable = editable
        if (field.text != text) {
            applying = true
            try { field.text = text; field.caretPosition = text.length } finally { applying = false }
        }
        if (focusKey != session) { focusKey = session; field.requestFocusInWindow() }
    }
    fun release() { changed = { false }; focused = {}; focusKey = null }
}
