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
    fun capture(): Ticket = Ticket(epoch, sequence)
    fun accepts(ticket: Ticket): Boolean = ticket == capture()
}
