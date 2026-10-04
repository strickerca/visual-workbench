package com.visualworkbench.android.instructions

import android.content.Context
import android.os.Build
import android.text.Editable
import android.text.TextWatcher
import android.text.InputFilter
import android.view.MotionEvent
import android.view.inputmethod.InputMethodManager
import android.widget.EditText
import com.visualworkbench.shared.*

/** A real platform editor/input connection. Handwriting is opt-in per field;
 * provenance follows that explicit mode, never a guessed transcript. */
internal class InstructionFieldView(context: Context) : EditText(context) {
    var changed: (String) -> Boolean = { true }
    var unavailable: (String) -> Unit = {}
    var mode: InstructionEntryMethod = InstructionEntryMethod.PhoneKeyboard
    private var applying = false
    init {
        minLines = 4; maxLines = 12
        inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_MULTI_LINE or android.text.InputType.TYPE_TEXT_FLAG_CAP_SENTENCES
        importantForAutofill = IMPORTANT_FOR_AUTOFILL_NO_EXCLUDE_DESCENDANTS
        imeOptions = android.view.inputmethod.EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING or android.view.inputmethod.EditorInfo.IME_FLAG_NO_EXTRACT_UI
        filters = arrayOf(InputFilter { source, start, end, dest, dstart, dend ->
            val projected = dest.length - (dend - dstart) + (end - start)
            if (projected > 32768) dest.subSequence(dstart, dend)
            else {
                val next = dest.subSequence(0, dstart).toString() + source.subSequence(start, end) + dest.subSequence(dend, dest.length)
                if (instructionTextFits(next)) null else dest.subSequence(dstart, dend)
            }
        })
        if (Build.VERSION.SDK_INT >= 34) setAutoHandwritingEnabled(false)
        addTextChangedListener(object : TextWatcher {
            override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
            override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) = Unit
            override fun afterTextChanged(s: Editable?) { if (!applying) changed(s?.toString().orEmpty()) }
        })
    }
    fun apply(text: String, entry: InstructionEntryMethod, editable: Boolean) {
        isEnabled = editable; mode = entry
        if (Build.VERSION.SDK_INT >= 34) setAutoHandwritingEnabled(entry == InstructionEntryMethod.Handwriting)
        if (getText().toString() != text) {
            applying = true
            try { setText(text); setSelection(text.length) } finally { applying = false }
        }
    }
    fun handwritingAvailable(): Boolean = Build.VERSION.SDK_INT >= 34 &&
        try { context.getSystemService(InputMethodManager::class.java).isStylusHandwritingAvailable } catch (_: Exception) { false }
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (Build.VERSION.SDK_INT >= 34 && mode == InstructionEntryMethod.Handwriting && event.actionMasked == MotionEvent.ACTION_DOWN &&
            (event.getToolType(0) == MotionEvent.TOOL_TYPE_STYLUS || event.getToolType(0) == MotionEvent.TOOL_TYPE_ERASER) && handwritingAvailable()) {
            requestFocus()
            try { context.getSystemService(InputMethodManager::class.java).startStylusHandwriting(this) }
            catch (_: Exception) { unavailable("The active keyboard could not start handwriting. Reopen its handwriting settings or use Keyboard; the draft is preserved.") }
        }
        return super.onTouchEvent(event)
    }
}
