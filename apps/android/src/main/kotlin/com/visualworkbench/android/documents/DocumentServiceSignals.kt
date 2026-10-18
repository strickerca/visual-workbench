package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CompletableDeferred

/** Framework callbacks are notifications, not interchangeable ownership proofs.
 * A normal one-job process may disconnect immediately after writing its result.
 * Only death (or proof that no job/FD was ever dispatched) permits retirement. */
internal class DocumentServiceSignals {
    private val sent = AtomicBoolean(false)
    val dispatched: Boolean get() = sent.get()
    val retired = CompletableDeferred<Unit>()
    val finished = CompletableDeferred<Unit>()
    val failed = CompletableDeferred<DocumentFailureKind>()
    fun dispatching() { sent.set(true) } // Before send: delivery can race a send error.
    fun resultReady() { finished.complete(Unit) }
    fun processDied() { retired.complete(Unit) }
    fun disconnected() { if (!sent.get()) failed.complete(DocumentFailureKind.Isolation) }
    fun failure(kind: DocumentFailureKind) { failed.complete(kind) }
}
