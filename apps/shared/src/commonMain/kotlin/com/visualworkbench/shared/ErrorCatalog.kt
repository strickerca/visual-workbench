package com.visualworkbench.shared

/** What to tell the person: what happened, and what they can do about it. */
public data class ProblemAdvice(public val whatHappened: String, public val whatToDo: String)

/** One place that turns a failure into plain language. The optional cause is
 * the core's own reason (see [CoreEventLog.latestFailure]); it only ever
 * selects a more specific message and is never shown as-is. */
public object ErrorCatalog {
    public fun core(kind: CoreFailureKind, cause: String? = null): ProblemAdvice {
        specific(cause)?.let { return it }
        return when (kind) {
            CoreFailureKind.Invalid -> ProblemAdvice(
                "That change could not be applied.",
                "Nothing was lost. Undo your last step or try the change again.",
            )
            CoreFailureKind.Backpressure -> ProblemAdvice(
                "The app is still finishing earlier work.",
                "Wait a moment and try again.",
            )
            CoreFailureKind.Closed -> ProblemAdvice(
                "This project was closed.",
                "Open the project again; your saved work is still there.",
            )
            CoreFailureKind.Cancelled -> ProblemAdvice(
                "That was cancelled before it finished.",
                "Nothing was changed. Start it again when you are ready.",
            )
            CoreFailureKind.Storage -> ProblemAdvice(
                "The project could not be saved or read.",
                "Check free space on this device, then reopen the project.",
            )
            CoreFailureKind.Unsupported -> ProblemAdvice(
                "This file uses a feature the app cannot handle yet.",
                "Export it in a common format such as PNG and import that.",
            )
            CoreFailureKind.Worker -> ProblemAdvice(
                "Something went wrong inside the app.",
                "Close and reopen the project. If it happens again, send a problem report.",
            )
            CoreFailureKind.Stroke -> ProblemAdvice(
                "That pen stroke could not be saved.",
                "Draw it again. If it keeps happening, send a problem report.",
            )
            CoreFailureKind.Raster -> ProblemAdvice(
                "The image could not be processed.",
                "If the image has no color profile, choose sRGB and try again.",
            )
            CoreFailureKind.ImageCodecUnavailable -> ProblemAdvice(
                "This device has no decoder for that image type.",
                "Install the system image extension for it, or convert the image to PNG.",
            )
        }
    }

    public fun session(kind: SessionFailureKind): ProblemAdvice = when (kind) {
        SessionFailureKind.Authentication -> ProblemAdvice(
            "The other device could not be verified.",
            "Pair the two devices again by scanning the code.",
        )
        SessionFailureKind.Expired -> ProblemAdvice(
            "That pairing code has expired.",
            "Show a new code on the computer and scan it.",
        )
        SessionFailureKind.LockedOut -> ProblemAdvice(
            "Pairing is paused after several failed attempts.",
            "Wait ten minutes, then try again.",
        )
        SessionFailureKind.Declined -> ProblemAdvice(
            "Pairing was declined.",
            "Nothing was changed. Start pairing again if you meant to accept.",
        )
        SessionFailureKind.Timeout, SessionFailureKind.Transport -> ProblemAdvice(
            "The other device could not be reached.",
            "Check that both are on the same Wi-Fi, or that USB tethering is on.",
        )
        SessionFailureKind.Storage -> ProblemAdvice(
            "The pairing could not be saved.",
            "Your projects are unchanged. Check free space and try again.",
        )
        SessionFailureKind.Backpressure -> ProblemAdvice(
            "The connection is busy.",
            "Wait for the current transfer to finish and try again.",
        )
        SessionFailureKind.Cancelled -> ProblemAdvice(
            "That was cancelled before it finished.",
            "Nothing was changed.",
        )
        SessionFailureKind.Closed -> ProblemAdvice(
            "The connection was closed.",
            "Connect again; your saved work is intact.",
        )
        else -> ProblemAdvice(
            "The connection could not be completed.",
            "Your saved work is intact. Try connecting again.",
        )
    }

    private fun specific(cause: String?): ProblemAdvice? = when (cause) {
        "low disk space", "storage full", "storage quota exceeded" -> ProblemAdvice(
            "This device is out of storage space.",
            "Free up some space, then try again. Your saved work is intact.",
        )
        "permission denied", "read-only file system" -> ProblemAdvice(
            "The app is not allowed to write to that location.",
            "Choose a different folder, or allow access to this one.",
        )
        "file not found" -> ProblemAdvice(
            "A file the project needs is missing.",
            "If you moved the project, move it back or open it from its new location.",
        )
        "exhausted order interval" -> ProblemAdvice(
            "This layer cannot hold any more items.",
            "Add a new layer and continue drawing there.",
        )
        "store busy" -> ProblemAdvice(
            "The project is open somewhere else.",
            "Close the other window or app that has this project open.",
        )
        else -> null
    }
}
