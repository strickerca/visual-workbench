package com.visualworkbench.shared

import java.util.concurrent.atomic.AtomicBoolean

/** The same monitor covers construction/publication and close's seal/snapshot.
 * Seal retains the actual owner for Pending retries; suspension happens outside
 * this monitor. The closed scalar remains the link's existing admission fence. */
internal class RemoteOwnerPublication<T:Any>(private val closed:AtomicBoolean,
    private val isRetired:(T)->Boolean) {
    private var owner:T?=null
    @Synchronized fun acquire(checkParent:()->Unit,create:()->T):T {
        checkParent()
        if(closed.get())throw SessionFailure(SessionFailureKind.Closed)
        val current=owner
        if(current!=null&&!isRetired(current))return current
        return create().also{owner=it}
    }
    @Synchronized fun seal():T? {closed.set(true);return owner}
}
