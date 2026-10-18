package com.visualworkbench.shared

/** Display projection only; native/store owners remain the currentness and CAS
 * authority. It never interprets an agent response as an owner decision. */
public sealed interface VerificationReviewOutcome {
    public data object AwaitingOwner : VerificationReviewOutcome
    public data object AllPass : VerificationReviewOutcome
    public data class Failures(public val originalNumbers: List<UInt>) : VerificationReviewOutcome
}

/** The UI retains this exact request after an uncertain reply. Its operation ID
 * must be retried unchanged, or discarded only after an explicit reload. */
public data class VerificationOwnerChange(public val pair: VerificationPair, public val expectedVersion: ULong,
    public val operationId: String, public val originalNumber: UInt, public val verdict: VerificationVerdict,
    public val note: String) {
    override fun toString(): String = "VerificationOwnerChange([PRIVATE])"
}

/** Bounded deterministic UI reducer. It owns no project, coroutine, review
 * store, or close protocol. The caller serializes it with its existing gate. */
public class VerificationInteraction {
    public var snapshot: VerificationReviewSnapshot? = null
        private set
    public var pending: VerificationOwnerChange? = null
        private set

    /** Explicit reload reconciles uncertain saves and discards the old request. */
    public fun loaded(pair: VerificationPair, value: VerificationReviewSnapshot): Unit {
        val next = ownedVerificationSnapshot(value, pair, PackageFailureKind.Integrity)
        snapshot = next
        pending = null
    }

    /** Call only from the manual owner's review action. An outstanding request
     * remains unchanged until its exact reply is accepted or a reload occurs. */
    public fun ownerChange(operationId: String, originalNumber: UInt, verdict: VerificationVerdict,
        note: String): VerificationOwnerChange {
        if (pending != null) throw PackageFailure(PackageFailureKind.Busy)
        val value = snapshot ?: throw PackageFailure(PackageFailureKind.Stale)
        validateVerificationChange(value.version, operationId, originalNumber, note)
        if (originalNumber > value.markerCount) throw PackageFailure(PackageFailureKind.Invalid)
        if (value.version == Long.MAX_VALUE.toULong()) throw PackageFailure(PackageFailureKind.Limit)
        val notes = value.markers.sumOf { if (it.originalNumber == originalNumber) 0 else verificationNoteBytes(it.note) }
        if (notes + verificationNoteBytes(note) > VERIFICATION_MAX_NOTES) throw PackageFailure(PackageFailureKind.Limit)
        return VerificationOwnerChange(value.pair, value.version, operationId, originalNumber, verdict, note).also { pending = it }
    }

    /** A stale, malformed or unrelated reply leaves the exact pending request
     * available for retry; it never advances local review state. */
    public fun changed(request: VerificationOwnerChange, value: VerificationReviewSnapshot): Unit {
        if (pending != request) throw PackageFailure(PackageFailureKind.Stale)
        val previous = snapshot ?: throw PackageFailure(PackageFailureKind.Stale)
        val next = ownedVerificationSnapshot(value, request.pair, PackageFailureKind.Integrity)
        if (next.projectId != previous.projectId || next.markerCount != previous.markerCount ||
            next.version != request.expectedVersion + 1uL) throw PackageFailure(PackageFailureKind.Integrity)
        for ((old, marker) in previous.markers.zip(next.markers)) {
            val expected = if (marker.originalNumber == request.originalNumber)
                old.copy(verdict = request.verdict, note = request.note) else old
            if (marker != expected) throw PackageFailure(PackageFailureKind.Integrity)
        }
        snapshot = next
        pending = null
    }

    public fun outcome(): VerificationReviewOutcome = snapshot?.let(::verificationReviewOutcome)
        ?: VerificationReviewOutcome.AwaitingOwner
}

/** Snapshot shape is validated; freshness still requires the native compile. */
public fun verificationReviewOutcome(value: VerificationReviewSnapshot): VerificationReviewOutcome {
    val snapshot = ownedVerificationSnapshot(value)
    if (snapshot.markers.any { it.verdict == VerificationVerdict.Pending }) return VerificationReviewOutcome.AwaitingOwner
    val failed = snapshot.markers.filter { it.verdict == VerificationVerdict.Fail }.map { it.originalNumber }
    return if (failed.isEmpty()) VerificationReviewOutcome.AllPass
        else VerificationReviewOutcome.Failures(VerificationReadOnlyList(failed))
}

internal const val VERIFICATION_MAX_MARKERS: Int = 512
internal const val VERIFICATION_MAX_NOTE: Int = 32 * 1024
internal const val VERIFICATION_MAX_NOTES: Int = 128 * 1024
internal const val VERIFICATION_HISTORY_MAX_ROWS: Int = 8
internal const val VERIFICATION_HISTORY_MAX_NOTES: Int = VERIFICATION_HISTORY_MAX_ROWS * VERIFICATION_MAX_NOTES
internal const val VERIFICATION_HISTORY_PREFIX: String = "vw.verification-review.v1."

/** Common read-only storage with a private copy; a JVM MutableList cast cannot
 * mutate an admitted snapshot or measured translation set after validation. */
private class VerificationReadOnlyList<T>(values: List<T>) : AbstractList<T>() {
    private val values: List<T> = values.toList()
    override val size: Int get() = values.size
    override fun get(index: Int): T = values[index]
}

internal fun verificationId(value: String): Boolean = value.length == 36 && value[14] == '7' &&
    value[19] in "89ab" && value.indices.all { index ->
        if (index == 8 || index == 13 || index == 18 || index == 23) value[index] == '-'
        else value[index] in '0'..'9' || value[index] in 'a'..'f'
    }

internal fun validateVerificationPair(value: VerificationPair, kind: PackageFailureKind = PackageFailureKind.Invalid): Unit {
    fun key(key: VerificationPackageKey): Boolean = verificationId(key.packageId) &&
        key.target in listOf("claude", "openai", "gemini", "generic") && hash(key.manifestSha256)
    if (!key(value.original) || !key(value.after) || value.original.packageId == value.after.packageId ||
        !verificationId(value.receiptId) || !hash(value.receiptBlake3)) throw PackageFailure(kind)
}

private fun hash(value: String): Boolean = value.length == 64 && value.all { it in '0'..'9' || it in 'a'..'f' }

internal fun verificationNoteBytes(value: String, kind: PackageFailureKind = PackageFailureKind.Invalid): Int {
    if ('\u0000' in value) throw PackageFailure(kind)
    if (value.length > VERIFICATION_MAX_NOTE) throw PackageFailure(PackageFailureKind.Limit)
    var index = 0
    while (index < value.length) {
        val char = value[index++]
        if (char in '\uD800'..'\uDBFF') {
            if (index == value.length || value[index++] !in '\uDC00'..'\uDFFF') throw PackageFailure(kind)
        } else if (char in '\uDC00'..'\uDFFF') throw PackageFailure(kind)
    }
    return value.encodeToByteArray().size.also { if (it > VERIFICATION_MAX_NOTE) throw PackageFailure(PackageFailureKind.Limit) }
}

private fun <T> boundedVerificationCopy(values: List<T>, kind: PackageFailureKind,
    maximum: Int = VERIFICATION_MAX_MARKERS): List<T> {
    if (values.size > maximum) throw PackageFailure(kind)
    val copy = ArrayList<T>(values.size)
    for (value in values) {
        if (copy.size == maximum) throw PackageFailure(kind)
        copy.add(value)
    }
    return copy
}

internal fun validateVerificationChange(expectedVersion: ULong, operationId: String, originalNumber: UInt,
    note: String): Unit {
    if (expectedVersion > Long.MAX_VALUE.toULong() || !verificationId(operationId) ||
        originalNumber !in 1u..VERIFICATION_MAX_MARKERS.toUInt()) throw PackageFailure(PackageFailureKind.Invalid)
    verificationNoteBytes(note)
}

internal fun ownedVerificationSnapshot(value: VerificationReviewSnapshot, pair: VerificationPair = value.pair,
    kind: PackageFailureKind = PackageFailureKind.Invalid): VerificationReviewSnapshot {
    validateVerificationPair(pair, kind)
    if (value.pair != pair || !verificationId(value.projectId) || value.version > Long.MAX_VALUE.toULong() ||
        value.markerCount > VERIFICATION_MAX_MARKERS.toUInt() || value.markers.size != value.markerCount.toInt())
        throw PackageFailure(kind)
    val markers = boundedVerificationCopy(value.markers, kind)
    if (markers.size != value.markerCount.toInt()) throw PackageFailure(kind)
    var noteBytes = 0
    for ((index, marker) in markers.withIndex()) {
        if (marker.originalNumber != (index + 1).toUInt()) throw PackageFailure(kind)
        noteBytes += verificationNoteBytes(marker.note, kind)
        if (noteBytes > VERIFICATION_MAX_NOTES) throw PackageFailure(PackageFailureKind.Limit)
    }
    // Match the admitted store contract: zero is the valid fresh review, before
    // any decision or note has been saved. It is not an unavailable sentinel.
    if (value.version == 0uL && markers.any { it.verdict != VerificationVerdict.Pending || it.note.isNotEmpty() })
        throw PackageFailure(kind)
    return value.copy(markers = VerificationReadOnlyList(markers))
}

internal fun ownedVerificationTranslations(expected: VerificationReviewSnapshot,
    values: List<VerificationMarkerTranslation>): List<VerificationMarkerTranslation> {
    if (values.size > VERIFICATION_MAX_MARKERS) throw PackageFailure(PackageFailureKind.Limit)
    val failed = expected.markers.filter { it.verdict == VerificationVerdict.Fail }.map { it.originalNumber }
    val owned = boundedVerificationCopy(values, PackageFailureKind.Limit)
    if (expected.markers.any { it.verdict == VerificationVerdict.Pending } || owned.size != failed.size ||
        owned.any { !it.dx.isFinite() || !it.dy.isFinite() } ||
        owned.map { it.originalNumber }.sorted() != failed) throw PackageFailure(PackageFailureKind.Invalid)
    return VerificationReadOnlyList(owned.sortedBy { it.originalNumber })
}

private fun verificationHistoryCursor(value: String): Boolean =
    value.length == VERIFICATION_HISTORY_PREFIX.length + 64 && value.startsWith(VERIFICATION_HISTORY_PREFIX) &&
        hash(value.substring(VERIFICATION_HISTORY_PREFIX.length))

internal fun validateVerificationHistoryRequest(after: String?, limit: UInt): Unit {
    if (limit !in 1u..VERIFICATION_HISTORY_MAX_ROWS.toUInt() || after?.let { !verificationHistoryCursor(it) } == true)
        throw PackageFailure(PackageFailureKind.Invalid)
}

/** Defends the decoded API boundary. Only native storage has the encoded row
 * bytes and key binding needed to enforce 2 MiB and last-returned-key identity. */
internal fun ownedVerificationHistoryPage(value: VerificationReviewHistoryPage, projectId: String,
    after: String? = null, limit: UInt = 8u): VerificationReviewHistoryPage {
    validateVerificationHistoryRequest(after, limit)
    if (!verificationId(projectId)) throw PackageFailure(PackageFailureKind.Integrity)
    val input = boundedVerificationCopy(value.snapshots, PackageFailureKind.Integrity, limit.toInt())
    val cursor = value.nextCursor
    if (cursor != null && (!verificationHistoryCursor(cursor) || input.isEmpty() || (after != null && cursor <= after)))
        throw PackageFailure(PackageFailureKind.Integrity)
    var noteBytes = 0
    val snapshots = input.map { snapshot ->
        val owned = ownedVerificationSnapshot(snapshot, kind = PackageFailureKind.Integrity)
        if (owned.version == 0uL || owned.projectId != projectId) throw PackageFailure(PackageFailureKind.Integrity)
        for (marker in owned.markers) {
            noteBytes += verificationNoteBytes(marker.note, PackageFailureKind.Integrity)
            if (noteBytes > VERIFICATION_HISTORY_MAX_NOTES) throw PackageFailure(PackageFailureKind.Limit)
        }
        owned
    }
    if (snapshots.map { Triple(it.pair, it.projectId, it.markerCount) }.distinct().size != snapshots.size)
        throw PackageFailure(PackageFailureKind.Integrity)
    return VerificationReviewHistoryPage(VerificationReadOnlyList(snapshots), cursor)
}
