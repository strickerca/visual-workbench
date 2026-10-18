package com.visualworkbench.android.airactions

import com.visualworkbench.shared.Camera

/** Local view memory only: never restores onto another page or viewport. */
internal data class AirActionViewOwner(val projectEpoch: Long, val documentId: String, val page: UInt,
    val width: Double, val height: Double)

internal class AirActionViewRestore {
    private data class Saved(val owner: AirActionViewOwner, val before: Camera, val fitted: Camera)
    private var saved: Saved? = null

    fun remember(owner: AirActionViewOwner, before: Camera, fitted: Camera) { saved = Saved(owner, before, fitted) }
    fun take(owner: AirActionViewOwner, current: Camera): Camera? {
        val previous = saved
        saved = null
        return previous?.takeIf { it.owner == owner && it.fitted == current }?.before
    }
    fun clear() { saved = null }
}
