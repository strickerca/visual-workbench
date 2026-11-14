package com.visualworkbench.shared

private const val MAX_CORE_EVENTS: Int = 512
private const val MAX_CORE_TEXT: Int = 64
private const val MAX_CORE_NUMBER: Long = 9_007_199_254_740_991L

/** What a core event's number means. */
public enum class CoreEventKind { Failure, DurationMicros, Count }

/** One diagnostic event from the Rust core. `area` and `name` are fixed
 * strings compiled into the core; this type still refuses anything outside a
 * narrow lower-case alphabet so a path, identifier or user text can never be
 * shown or exported through it. */
public data class CoreEvent(
    public val sequence: Long,
    public val elapsedMillis: Long,
    public val kind: CoreEventKind,
    public val area: String,
    public val name: String,
    public val value: Long,
) {
    init {
        require(sequence in 1L..MAX_CORE_NUMBER && elapsedMillis in 0L..MAX_CORE_NUMBER && value in 0L..MAX_CORE_NUMBER)
        require(isCoreEventText(area) && isCoreEventText(name))
    }
}

/** The core's retained events and how many older ones it has discarded. */
public data class CoreEventLog(public val events: List<CoreEvent>, public val dropped: Long) {
    init {
        require(events.size <= MAX_CORE_EVENTS && dropped in 0L..MAX_CORE_NUMBER)
        require(events.zipWithNext().all { (before, after) -> before.sequence < after.sequence })
    }

    /** The newest failure, which is the best available reason for an error the
     * app has just been handed. */
    public fun latestFailure(): CoreEvent? = events.lastOrNull { it.kind == CoreEventKind.Failure }

    /** Plain lines for a "report a problem" preview: numbers and fixed words only. */
    public fun reportLines(): List<String> = events.map { event ->
        val seconds = event.elapsedMillis / 1000L
        val millis = (event.elapsedMillis % 1000L).toString().padStart(3, '0')
        when (event.kind) {
            CoreEventKind.Failure -> "$seconds.$millis s  ${event.area}: ${event.name}"
            CoreEventKind.DurationMicros -> "$seconds.$millis s  ${event.area}: ${event.name} took ${event.value / 1000L}.${(event.value % 1000L).toString().padStart(3, '0')} ms"
            CoreEventKind.Count -> "$seconds.$millis s  ${event.area}: ${event.name} = ${event.value}"
        }
    }
}

/** Lower-case letters, digits, space, dot, underscore and hyphen; 1 to 64 characters. */
public fun isCoreEventText(text: String): Boolean =
    text.length in 1..MAX_CORE_TEXT && text.all { it in 'a'..'z' || it in '0'..'9' || it == ' ' || it == '.' || it == '_' || it == '-' }
