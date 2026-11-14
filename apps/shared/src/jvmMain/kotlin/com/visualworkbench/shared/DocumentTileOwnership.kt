package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.util.IdentityHashMap
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*

internal class DocumentForeignBudget(private val limit: Long) {
    private var used = 0L
    init { require(limit > 0) }
    @Synchronized fun reserve(bytes: Long): AutoCloseable {
        if (bytes <= 0 || bytes > limit - used) throw DocumentFailure(DocumentFailureKind.Limit)
        used += bytes
        return object : AutoCloseable {
            private var held = true
            override fun close() = synchronized(this@DocumentForeignBudget) { if (held) { used -= bytes; held = false } }
        }
    }
    @Synchronized fun retainedBytes(): Long = used
}

internal class DocumentImage<T>(val value: T, private val reservation: AutoCloseable, private val dispose: () -> Unit) {
    private var references = 1 // Cache/staging owner; frames acquire independent references.
    @Synchronized fun retain() { check(references > 0); references++ }
    @Synchronized fun release() {
        check(references > 0)
        if (references == 1) { dispose(); reservation.close(); references = 0 } else references--
    }
}

/** A published frame owns this lease until its LAST platform draw reader has
 * retired. UI state replacement alone is not that proof. */
internal class PreparedTile<T>(val source: DocumentDisplayTile, val binding: DocumentFrameBinding, private val image: DocumentImage<T>) : AutoCloseable {
    private var held = true
    val value: T get() = synchronized(this) { check(held); image.value }
    @Synchronized override fun close() { if (held) { image.release(); held = false } }
}

internal class DocumentTileImages<T>(
    budgetBytes: Long,
    private val convert: (DocumentDisplayTile, () -> Boolean) -> Pair<T, () -> Unit>,
) : AutoCloseable {
    private val budget = DocumentForeignBudget(budgetBytes)
    private val images = IdentityHashMap<DocumentDisplayTile, DocumentImage<T>>()
    private var binding: DocumentFrameBinding? = null
    private val quarantine = mutableListOf<PreparedTile<T>>()
    @Synchronized fun prepare(frame: DocumentDisplayFrame, stillCurrent: () -> Boolean): List<PreparedTile<T>> {
        if (quarantine.isNotEmpty()) throw DocumentFailure(DocumentFailureKind.Isolation)
        validateDocumentDisplayFrame(frame, frame.binding)
        for (tile in frame.tiles) {
            val scale = tile.scale.toFloat(); val inverse = (1.0 / tile.scale).toFloat()
            if (!scale.isFinite() || scale <= 0f || !inverse.isFinite() || inverse <= 0f ||
                !tile.origin.x.toFloat().isFinite() || !tile.origin.y.toFloat().isFinite())
                throw DocumentFailure(DocumentFailureKind.Verification)
        }
        current(stillCurrent)
        if (binding != frame.binding) { clear(); binding = frame.binding }
        val wanted = IdentityHashMap<DocumentDisplayTile, Boolean>()
        for (tile in frame.tiles) {
            if (wanted.put(tile, true) != null) throw DocumentFailure(DocumentFailureKind.Verification)
        }
        val retired = images.keys.filter { !wanted.containsKey(it) }
        for (tile in retired) { images.getValue(tile).release(); images.remove(tile) }
        val result = ArrayList<PreparedTile<T>>(frame.tiles.size)
        try {
            for (tile in frame.tiles) {
                current(stillCurrent)
                val image = images[tile] ?: run {
                    // Four pixel copies plus bounded object/row bookkeeping are
                    // reserved for the image's whole lifetime, conservatively.
                    // This is separate from native engine/source reservations.
                    val reservation = budget.reserve(tile.rgba.size.toLong() * 4 + 4096)
                    val created = try { convert(tile, stillCurrent) } catch (error: Throwable) { reservation.close(); throw error }
                    DocumentImage(created.first, reservation, created.second).also { images[tile] = it }
                }
                image.retain(); result += PreparedTile(tile, frame.binding, image)
            }
            current(stillCurrent)
            return result
        } catch (error: Throwable) {
            for (tile in result) try { tile.close() } catch (_: Exception) { quarantine += tile }
            clear()
            if (quarantine.isNotEmpty()) throw DocumentFailure(DocumentFailureKind.Isolation)
            throw error
        }
    }
    /** Capture the acquired frame BEFORE dispatcher return, so prompt
     * cancellation cannot discard image leases that the caller never saw. */
    suspend fun prepareOwned(frame: DocumentDisplayFrame, stillCurrent: () -> Boolean): List<PreparedTile<T>> {
        val handoff = AtomicReference<List<PreparedTile<T>>?>(null)
        val caller = currentCoroutineContext()[Job]
        try {
            val result = withContext(Dispatchers.Default) {
                prepare(frame) { caller?.isActive != false && stillCurrent() }.also(handoff::set)
            }
            currentCoroutineContext().ensureActive()
            if (!stillCurrent()) throw DocumentFailure(DocumentFailureKind.Stale)
            handoff.set(null)
            return result
        } catch (error: Exception) {
            if (caller?.isActive == false) throw CancellationException("Document bitmap preparation cancelled")
            throw error
        } finally { handoff.getAndSet(null)?.let { abandon(it) } }
    }
    @Synchronized private fun abandon(tiles: List<PreparedTile<T>>) {
        for (tile in tiles) try { tile.close() } catch (_: Exception) { quarantine += tile }
        if (quarantine.isNotEmpty()) throw DocumentFailure(DocumentFailureKind.Isolation)
    }
    @Synchronized fun retainedBytes(): Long = budget.retainedBytes()
    @Synchronized override fun close() {
        for (index in quarantine.indices.reversed()) try { quarantine[index].close(); quarantine.removeAt(index) } catch (_: Exception) { }
        clear()
        if (quarantine.isNotEmpty()) throw DocumentFailure(DocumentFailureKind.Isolation)
        binding = null
    }
    private fun clear() {
        // If a disposal fails, retain that cache owner for joined close retry.
        var failure: Exception? = null
        for (tile in images.keys.toList()) try { images.getValue(tile).release(); images.remove(tile) } catch (error: Exception) { failure = error }
        if (failure != null) throw DocumentFailure(DocumentFailureKind.Isolation)
    }
    private fun current(check: () -> Boolean) { if (!check()) throw DocumentFailure(DocumentFailureKind.Stale) }
}

internal fun documentArgb(rgba: ByteArray, offset: Int): Int =
    ((rgba[offset + 3].toInt() and 255) shl 24) or ((rgba[offset].toInt() and 255) shl 16) or
        ((rgba[offset + 1].toInt() and 255) shl 8) or (rgba[offset + 2].toInt() and 255)
