package com.visualworkbench.shared

import kotlinx.coroutines.CompletableDeferred

/** Publisher and renderer hold separate leases. A renderer releases only after
 * its actual readers retire; replacing UI state never releases a reader. */
public class DocumentDrawFrame<out T : AutoCloseable>(public val binding: DocumentFrameBinding, tiles: List<T>) : AutoCloseable {
    private val tiles: MutableList<T> = tiles.toMutableList()
    private var publisher: Boolean = true
    private var references: Int = 1
    private val remaining: MutableList<T> = tiles.toMutableList()
    private val retirement: CompletableDeferred<Unit> = CompletableDeferred()
    public val available:Boolean get()=synchronized(this){publisher}
    public val retired: Boolean get() = synchronized(this) { retirement.isCompleted }
    @Synchronized public fun acquire(): DocumentDrawLease<T>? {
        if (!publisher) return null
        references++
        return DocumentDrawLease(this, tiles)
    }
    @Synchronized internal fun retainReader(): Unit { check(references > 0); references++ }
    @Synchronized internal fun releaseReader(): Unit { check(references > 0); references--; disposeIfRetired() }
    @Synchronized public override fun close(): Unit {
        if (publisher) { publisher = false; references-- }
        disposeIfRetired()
    }
    private fun disposeIfRetired(): Unit {
        if (references != 0) return
        for (index in remaining.indices.reversed()) try { remaining[index].close(); remaining.removeAt(index) } catch (_: Exception) { }
        if (remaining.isEmpty()) {tiles.clear();retirement.complete(Unit)}
        else throw DocumentFailure(DocumentFailureKind.Isolation)
    }
    public suspend fun awaitRetired(): Unit { retirement.await() }
}
public class DocumentDrawLease<out T : AutoCloseable> internal constructor(private val owner: DocumentDrawFrame<T>, private val tiles: List<T>) : AutoCloseable {
    private var held: Boolean = true
    public val binding: DocumentFrameBinding get() = owner.binding
    public fun <R> read(block: (List<T>) -> R): R? {
        synchronized(this) { if (!held) return null; owner.retainReader() }
        try { return block(tiles) } finally { owner.releaseReader() }
    }
    @Synchronized public override fun close(): Unit { if (held) { held = false; owner.releaseReader() } }
}
