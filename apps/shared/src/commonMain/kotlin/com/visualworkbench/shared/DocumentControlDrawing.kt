package com.visualworkbench.shared

/** An explicit layer choice remains explicit when a peer hides/locks/deletes it.
 * Null means the editor's original default, before the controls are activated.
 * Layer kinds are immutable; the controller only assigns annotation IDs. */
public fun documentControlDrawingLayer(document: DocumentSnapshot, chosen: String?): LayerInfo? =
    if (chosen != null) document.layers.firstOrNull { it.id == chosen && it.visible && !it.locked }
    else document.layers.firstOrNull { it.visible && !it.locked && it.blend == "normal" }
