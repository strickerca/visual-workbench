package com.visualworkbench.android.airactions

import android.view.KeyEvent
import android.view.Window

/** Public platform callback hook, installed after Activity creation. It observes
 * actual events before the previous callback's normal view/Activity dispatch.
 * It never calls ComponentActivity's library-restricted dispatchKeyEvent API. */
internal class AirActionWindowKeyHook(private val window: Window, receive: (KeyEvent) -> Boolean) {
    private val previous = window.callback
    private var active = true
    private var installed: Window.Callback? = null

    init {
        val original = previous
        if (original != null) {
            val callback = object : Window.Callback by original {
                override fun dispatchKeyEvent(event: KeyEvent): Boolean =
                    dispatchAirKeyBeforeFallback(event, active, receive, original::dispatchKeyEvent)
            }
            installed = callback
            window.callback = callback
        }
    }

    fun close() {
        if (!active) return
        active = false
        if (installed != null && window.callback === installed) window.callback = previous
        installed = null
    }
}

/** The same actual event reaches either the consuming observer or exactly one
 * normal fallback. Retired callbacks cannot observe later events through a chain. */
internal fun <T> dispatchAirKeyBeforeFallback(event: T, active: Boolean,
    receive: (T) -> Boolean, fallback: (T) -> Boolean): Boolean {
    if (active && receive(event)) return true
    return fallback(event)
}
