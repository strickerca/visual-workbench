package com.visualworkbench.android.session

import kotlinx.coroutines.CancellationException

/** Main-confined event fence. A completed async snapshot may publish only if
 * no newer event or project epoch has been observed since its capture.
 *
 * It also keeps a save pending until the document that save produced has been
 * adopted. The store announces a save on its own thread. When the
 * announcement lands while the save's own re-read is in flight, that read is
 * dropped, exactly as it always was, and the refresh the announcement queued
 * reads and adopts a moment later. Until then the editor still holds the old
 * document, so it must not look idle: the dropped read hands its save over
 * ([owed]) and the queued refresh settles it. Nothing is read or adopted more
 * often for this, and no lock is held longer; only the moment the pending
 * count drops moves. [pending] receives +1 for a save handed over and the
 * negative count when they are settled. */
internal class RefreshAdmission(private val pending: (Int) -> Unit = {}) {
    data class Ticket(val epoch: Long, val sequence: ULong)
    /** What a finished read may do: adopt its result; drop it because a newer
     * ordered change overtook it; or drop it because the project or page moved on. */
    enum class Verdict { Adopt, Overtaken, Dropped }
    private var epoch = 0L
    private var sequence = 0uL
    fun reset(): Ticket { epoch++; sequence = 0uL; covered = 0uL; claimed = 0uL; return capture() }
    fun observe(value: ULong): Boolean {
        if (value <= sequence) return false
        sequence = value
        return true
    }
    fun invalidate() { epoch++ } // Preserve ordered event sequence during page navigation.
    fun capture(): Ticket = Ticket(epoch, sequence)
    fun accepts(ticket: Ticket): Boolean = ticket == capture()
    /** The ticket's project and page still stand; only a newer ordered change
     * arrived while its read was in flight. */
    fun overtaken(ticket: Ticket): Boolean = ticket.epoch == epoch && ticket.sequence < sequence
    fun judge(ticket: Ticket): Verdict = if (accepts(ticket)) Verdict.Adopt else if (overtaken(ticket)) Verdict.Overtaken else Verdict.Dropped

    // Saves handed over to the queued refresh.
    /** Saves whose own read was overtaken and whose document is not adopted yet. */
    var owed = 0; private set
    /** Identifies the running queued-refresh loop; 0 when none runs. */
    private var serving = 0
    private var served = 0
    /** Every change up to this sequence has been read by a queued refresh that
     * took its ticket, or belongs to one that ended without reading. */
    private var covered = 0uL
    /** The sequence when the queued refresh in progress was started. */
    private var claimed = 0uL
    /** Whether a queued refresh is still going to take a ticket for what has
     * been observed. Only then can a dropped read be handed over to it. */
    private val queuedRefreshDue: Boolean get() = serving != 0 && sequence > covered

    /** A queued-refresh loop starts for the current project. The returned
     * token ends exactly this loop in [stopServing]. */
    fun serve(): Int { serving = ++served; covered = sequence; claimed = sequence; return serving }
    /** The loop, or the observer that feeds it, ended: nothing further will be
     * adopted for the saves handed over, so they stop counting as pending. */
    fun stopServing(token: Int) { if (token == serving) { serving = 0; settle() } }

    /** One refresh: one read and at most one adoption. [read] returns null
     * when it no longer belongs to the current project or page. [adopt] runs
     * in the same turn as the verdict, before anything else can be observed.
     * [queued] marks the refresh an announced change queued, as opposed to a
     * save's own re-read. */
    suspend fun <T : Any> refresh(queued: Boolean, read: suspend () -> T?, adopt: suspend (T) -> Unit): Verdict {
        if (queued) covered = sequence
        val ticket = capture()
        val value = read() ?: return Verdict.Dropped
        val verdict = judge(ticket)
        if (verdict == Verdict.Adopt) adopt(value)
        else if (verdict == Verdict.Overtaken && !queued && queuedRefreshDue) { owed++; pending(1) }
        return verdict
    }

    /** One turn of the queued-refresh loop around [refresh]. A turn that was
     * not itself overtaken settles the saves handed over: their document is
     * adopted, or can no longer be (the page moved on, or the read failed and
     * [failed] reported it). An overtaken turn leaves them: the change that
     * overtook it has queued the next turn. */
    suspend fun queuedTurn(failed: (Exception) -> Unit, refresh: suspend () -> Verdict) {
        claimed = sequence
        val verdict = try { refresh() }
            catch (cancel: CancellationException) { throw cancel }
            catch (error: Exception) { failed(error); Verdict.Dropped }
        // A turn that ended before taking its ticket still used up its request.
        if (claimed > covered) covered = claimed
        if (verdict != Verdict.Overtaken) settle()
    }

    private fun settle() { if (owed != 0) { val count = owed; owed = 0; pending(-count) } }
}
