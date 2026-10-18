package com.visualworkbench.shared

/** Local page navigation does not change the canonical revision. This separate
 * view fence rejects in-flight refreshes, including page A -> B -> A races. */
public class DocumentRefreshAdmission {
    public data class Ticket(public val generation: Long, public val documentId: String, public val page: UInt?)
    public data class Loaded(public val ticket: Ticket, public val snapshot: DocumentSnapshot)
    private var generation: Long = 0
    public fun invalidate(): Unit { generation++ }
    public fun capture(snapshot: DocumentSnapshot): Ticket = Ticket(generation,snapshot.documentId,snapshot.page)
    public fun accepts(ticket: Ticket, current: DocumentSnapshot?, incoming: DocumentSnapshot): Boolean =
        ticket.generation == generation && ticket.documentId == current?.documentId && ticket.page == current.page &&
            incoming.documentId == ticket.documentId && incoming.page == ticket.page
    /** Both app refresh paths use this same read/adoption boundary. Recheck
     * accepts() after any later suspension before adopting or rebinding tools. */
    public suspend fun load(requested: DocumentSnapshot, current: () -> DocumentSnapshot?,
        read: suspend () -> DocumentSnapshot): Loaded? {
        val ticket=capture(requested)
        val incoming=read()
        return if(accepts(ticket,current(),incoming)) Loaded(ticket,incoming) else null
    }
}
