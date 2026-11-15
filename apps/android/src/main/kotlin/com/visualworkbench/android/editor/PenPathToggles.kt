package com.visualworkbench.android.editor

/** Developer switches for the pen path. Each one changes behaviour that can
 * only be judged on the phone with a pen, so each is off until it has been
 * measured there. While a switch is off, the code it guards is not reached and
 * the app behaves exactly as it did before the switch existed. [key] is the
 * stored name and never changes; [title] and [effect] are shown to the owner. */
internal enum class PenPathToggle(val key: String, val title: String, val effect: String) {
    CanvasQuickRefresh("canvasQuickRefresh", "Canvas: quick refresh after a save",
        "After each saved stroke or edit only what changed is read back, instead of rebuilding every stroke in the document. The more strokes a document has, the more this saves between one stroke and the next."),
    CanvasInkBeforeSave("canvasInkBeforeSave", "Canvas: next stroke starts while the last one saves",
        "Ink for a new stroke no longer waits until the stroke before it is saved and refreshed, also not during the occasional slow recovery checkpoint, and a pen-down is no longer dropped while instructions are re-read after a save. Strokes are still saved in the order they were drawn."),
    CanvasInkOnly("canvasInkOnly", "Canvas: draw only the new stroke",
        "While a pen or marker stroke is in progress, the finished picture of the document is kept and only the new stroke is drawn over it, instead of redrawing every object for every pen sample. Highlighter strokes and strokes under another layer are drawn the old way."),
    RemoteUnbufferedPen("remoteUnbufferedPen", "Remote pen: samples at the pen's own rate",
        "Asks Android to deliver each pen sample as it is reported instead of once per display frame, so samples leave for the computer, and ghost ink appears, up to a frame sooner. Applies the next time the remote view opens."),
    RemoteExactTimes("remoteExactTimes", "Remote pen: exact sample times",
        "Each sample keeps the time the pen reported it instead of a time rounded to when the phone handled it. Steadier ghost ink and truer timings. Applies the next time the remote view opens."),
    RemoteLongBatches("remoteLongBatches", "Remote pen: keep drawing through a hitch",
        "A burst of more than 8 pen samples in one event is sent to the computer in paced groups of 8 instead of pausing input. Applies the next time the remote view opens."),
    RemoteTypedBatches("remoteTypedBatches", "Remote pen: fewer, typed calls to the core",
        "Pen samples that are waiting go to the core together as typed values instead of one text message each. Less work per sample; with the hitch switch it also takes bursts of up to 32 samples. The computer receives exactly the same data. Applies the next time the remote view opens."),
    RemoteQuickFrames("remoteQuickFrames", "Remote view: show each frame sooner",
        "The video decoder is watched for each finished frame instead of being checked every 4 ms, and is asked to run as for 60 frames a second where the phone supports it. The computer still sends 30 frames a second. Applies the next time the remote view opens."),
}

/** Immutable set of switched-on toggles. The default has none. */
internal data class PenPathToggles(private val enabled: Set<PenPathToggle> = emptySet()) {
    operator fun get(toggle: PenPathToggle): Boolean = toggle in enabled
    fun with(toggle: PenPathToggle, on: Boolean): PenPathToggles = PenPathToggles(if (on) enabled + toggle else enabled - toggle)
    val anyOn: Boolean get() = enabled.isNotEmpty()

    /** Stored form: one entry per switched-on toggle, by stable key. */
    fun encode(): Map<String, Boolean> = PenPathToggle.entries.filter { it in enabled }.associate { it.key to true }

    companion object {
        /** Only a stored literal `true` under a known key switches a toggle on.
         * Anything missing, unknown or malformed leaves it off. */
        fun decode(stored: Map<String, Any?>): PenPathToggles =
            PenPathToggles(PenPathToggle.entries.filterTo(LinkedHashSet()) { stored[it.key] == true })
    }
}
