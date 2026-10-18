package com.visualworkbench.android.airactions

import android.view.KeyEvent
import com.visualworkbench.shared.AirActionGesture
import com.visualworkbench.shared.AirActionKeyPhase

@Suppress("DEPRECATION") // Explicitly exercise rejection of the legacy callback.
internal object AirActionKeyContract {
    fun scenarios(): Map<String, () -> Unit> = linkedMapOf(
        "declared F1-F8 slots match specification order" to {
            check((KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F8).map {
                key(it)!!.gesture
            } == AirActionGesture.entries)
        },
        "unrelated and modified keys stay in the ordinary key route" to {
            for (code in listOf(KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_BACK, KeyEvent.KEYCODE_STYLUS_BUTTON_PRIMARY, KeyEvent.KEYCODE_F9)) check(key(code) == null)
            for (modifier in listOf(KeyEvent.META_ALT_ON, KeyEvent.META_CTRL_ON, KeyEvent.META_SHIFT_ON, KeyEvent.META_META_ON)) check(key(KeyEvent.KEYCODE_F1, meta = modifier) == null)
        },
        "repeat cancellation timestamps and ephemeral stream survive adaptation" to {
            val key = airActionKey(KeyEvent.KEYCODE_F3, KeyEvent.ACTION_UP, 9, 20, 21, 2, true, 0)!!
            check(key.phase == AirActionKeyPhase.Up && key.gesture == AirActionGesture.SwipeLeft)
            check(key.stream == 9 && key.downTimeMs == 20L && key.eventTimeMs == 21L && key.repeatCount == 2 && key.canceled)
        },
        "multiple is consumed and unknown action is not recognized" to {
            check(key(KeyEvent.KEYCODE_F1, KeyEvent.ACTION_MULTIPLE)!!.phase == AirActionKeyPhase.Multiple)
            check(key(KeyEvent.KEYCODE_F1, 99) == null)
        },
    )
    private fun key(code: Int, action: Int = KeyEvent.ACTION_DOWN, meta: Int = 0) = airActionKey(code, action, 1, 10, 10, 0, false, meta)
}
