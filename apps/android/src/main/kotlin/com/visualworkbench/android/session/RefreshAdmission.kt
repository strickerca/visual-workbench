package com.visualworkbench.android.session

/** Main-confined event fence. A completed async snapshot may publish only if
 * no newer event or project epoch has been observed since its capture. */
internal class RefreshAdmission {
    data class Ticket(val epoch: Long, val sequence: ULong)
    private var epoch = 0L
    private var sequence = 0uL
    fun reset(): Ticket { epoch++; sequence = 0uL; return capture() }
    fun observe(value: ULong): Boolean {
        if (value <= sequence) return false
        sequence = value
        return true
    }
    fun invalidate() { epoch++ } // Preserve ordered event sequence during page navigation.
    fun capture(): Ticket = Ticket(epoch, sequence)
    fun accepts(ticket: Ticket): Boolean = ticket == capture()
    /** The ticket's project and page still stand; only a newer ordered change
     * arrived while its read was in flight. That read may simply be repeated. */
    fun overtaken(ticket: Ticket): Boolean = ticket.epoch == epoch && ticket.sequence < sequence

    /** Reads under the fence and repeats a read that a newer ordered change
     * overtook, at most [attempts] times. The store announces a save on its own
     * thread, so that announcement can land while the save's own re-read is in
     * flight; giving up there would leave the saved edit out of the visible
     * document until a later refresh. [read] returns null when the read no
     * longer belongs to the current project or page, which ends the attempts.
     * The returned value was read with no newer event observed since its
     * capture, exactly as [accepts] requires; adopt it before suspending. */
    suspend fun <T : Any> readLatest(attempts: Int, read: suspend () -> T?): T? {
        repeat(attempts) {
            val ticket = capture()
            val value = read() ?: return null
            if (accepts(ticket)) return value
            if (!overtaken(ticket)) return null
        }
        return null
    }
}
