package com.visualworkbench.android.airactions

import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.KeyEvent
import com.visualworkbench.shared.AirActionGesture
import com.visualworkbench.shared.AirActionKeyEvent
import com.visualworkbench.shared.AirActionKeyPhase
import com.visualworkbench.shared.AirActionRouter

/** Matches only the eight declared, unmodified action keys. No MotionEvent is
 * converted into a KeyEvent, and no Bluetooth permission or vendor SDK is needed. */
@Suppress("DEPRECATION") // Consume legacy ACTION_MULTIPLE without treating it as a gesture.
internal fun airActionKey(keyCode: Int, action: Int, stream: Int, downTimeMs: Long,
    eventTimeMs: Long, repeatCount: Int, canceled: Boolean, metaState: Int): AirActionKeyEvent? {
    val modifiers = KeyEvent.META_SHIFT_ON or KeyEvent.META_SHIFT_LEFT_ON or KeyEvent.META_SHIFT_RIGHT_ON or
        KeyEvent.META_ALT_ON or KeyEvent.META_ALT_LEFT_ON or KeyEvent.META_ALT_RIGHT_ON or
        KeyEvent.META_CTRL_ON or KeyEvent.META_CTRL_LEFT_ON or KeyEvent.META_CTRL_RIGHT_ON or
        KeyEvent.META_META_ON or KeyEvent.META_META_LEFT_ON or KeyEvent.META_META_RIGHT_ON or
        KeyEvent.META_SYM_ON or KeyEvent.META_FUNCTION_ON
    if (metaState and modifiers != 0) return null
    val gesture = when (keyCode) {
        KeyEvent.KEYCODE_F1 -> AirActionGesture.SingleClick
        KeyEvent.KEYCODE_F2 -> AirActionGesture.DoubleClick
        KeyEvent.KEYCODE_F3 -> AirActionGesture.SwipeLeft
        KeyEvent.KEYCODE_F4 -> AirActionGesture.SwipeRight
        KeyEvent.KEYCODE_F5 -> AirActionGesture.SwipeUp
        KeyEvent.KEYCODE_F6 -> AirActionGesture.SwipeDown
        KeyEvent.KEYCODE_F7 -> AirActionGesture.CircleClockwise
        KeyEvent.KEYCODE_F8 -> AirActionGesture.CircleCounterclockwise
        else -> return null
    }
    val phase = when (action) {
        KeyEvent.ACTION_DOWN -> AirActionKeyPhase.Down
        KeyEvent.ACTION_UP -> AirActionKeyPhase.Up
        KeyEvent.ACTION_MULTIPLE -> AirActionKeyPhase.Multiple
        else -> return null
    }
    return AirActionKeyEvent(gesture, phase, stream, downTimeMs, eventTimeMs, repeatCount, canceled)
}

/** Activity-owned timer only; use the same main-thread router for all callbacks.
 * Key callbacks must delegate here exactly once, before the normal fallback.
 * A hardware keyboard can also emit F1-F8: delivery is not proof of a BLE pen. */
internal class AirActionKeys(private val router: AirActionRouter) {
    private val handler = Handler(Looper.getMainLooper())
    private var closed = false
    private val flush: Runnable = Runnable { router.advance(); schedule() }

    fun accept(event: KeyEvent): Boolean {
        val key = airActionKey(event.keyCode, event.action, event.deviceId, event.downTime,
            event.eventTime, event.repeatCount, event.isCanceled, event.metaState) ?: return false
        router.key(key)
        schedule()
        return true
    }

    /** Call after configure and barrel observations that can cancel a deadline. */
    fun schedule() {
        handler.removeCallbacks(flush)
        if (!closed) router.nextDeadlineMs?.let { handler.postAtTime(flush, maxOf(it, SystemClock.uptimeMillis())) }
    }

    fun interrupt() { handler.removeCallbacks(flush); router.interrupt() }
    fun close() { if (!closed) { closed = true; handler.removeCallbacks(flush); router.close() } }
}
