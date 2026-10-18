package com.visualworkbench.android.airactions

import com.visualworkbench.shared.AirActionBindings
import com.visualworkbench.shared.AirActionCommand
import com.visualworkbench.shared.AirActionGesture

/** Stable names; presence distinguishes disabled from an older missing slot.
 * Unknown names cannot turn a remap into an arbitrary command. */
internal fun decodeAirActionPreferences(values: Map<String, Any?>): AirActionBindings {
    var bindings = AirActionBindings()
    for (gesture in AirActionGesture.entries) {
        if (!values.containsKey(gesture.name)) continue
        val raw = values[gesture.name]
        bindings = bindings.remap(gesture, AirActionCommand.entries.firstOrNull { raw is String && it.name == raw })
    }
    return bindings
}

internal fun encodeAirActionPreferences(bindings: AirActionBindings): Map<String, String?> =
    bindings.snapshot().mapKeys { it.key.name }.mapValues { it.value?.name }
