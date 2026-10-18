package com.visualworkbench.shared

/** Exact identities from the checked catalog and retained comparison receipt. */
public data class VerificationPackageKey(public val packageId: String, public val target: String,
    public val manifestSha256: String)
public data class VerificationPair(public val original: VerificationPackageKey, public val after: VerificationPackageKey,
    public val receiptId: String, public val receiptBlake3: String)
public enum class VerificationVerdict { Pending, Pass, Fail }

/** Installation-local owner input. Notes must not enter logs or portable output. */
public data class VerificationMarkerReview(public val originalNumber: UInt, public val verdict: VerificationVerdict,
    public val note: String) {
    override fun toString(): String = "VerificationMarkerReview([PRIVATE])"
}
/** A fresh review has version zero, with every marker Pending and every note
 * empty. The first saved manual decision advances that exact version to one. */
public data class VerificationReviewSnapshot(public val pair: VerificationPair, public val projectId: String,
    public val markerCount: UInt, public val version: ULong, public val markers: List<VerificationMarkerReview>) {
    override fun toString(): String = "VerificationReviewSnapshot([PRIVATE])"
}

/** Translation measured against the exact after source by the private capture
 * workflow. Equal image dimensions alone do not establish this mapping. */
public data class VerificationMarkerTranslation(public val originalNumber: UInt, public val dx: Double,
    public val dy: Double)

/** Optional private owner capability borrowing existing project/catalog/inbox
 * owners. This is not an agent tool. Only an explicit manual owner decision may
 * call change; measured alignment neither supplies nor authorizes a verdict.
 * Cancellation settles native work; a saved change may need exact-ID retry. */
public interface WorkbenchVerification {
    public suspend fun review(pair: VerificationPair): VerificationReviewSnapshot
    public suspend fun change(pair: VerificationPair, expectedVersion: ULong, operationId: String,
        originalNumber: UInt, verdict: VerificationVerdict, note: String): VerificationReviewSnapshot
    /** Pending or stale reviews refuse. All-pass returns null after checking
     * current owners. Failed markers require exact measured translations and
     * are compiled normally over the after source, without private notes.
     * The default uses the existing native 256 MiB ceiling: normal package
     * compilation reserves over 160 MiB before source/codec work, in addition
     * to the verification caller's retained packages and canonical workspace.
     * Smaller explicit budgets remain bounded and may refuse compilation. */
    public suspend fun compileFailures(expected: VerificationReviewSnapshot,
        translations: List<VerificationMarkerTranslation>, packageId: String, createdAtMs: Long,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): WorkbenchCompiledPackage?
}

public expect fun verificationWorkflows(original: WorkbenchProject, after: WorkbenchProject,
    catalog: WorkbenchPackageCatalog, inbox: WorkbenchMcpInbox): WorkbenchVerification

/** Saved installation-local notes only. Reading history does not recover a
 * capture selector, grant, current source binding or authority to change it. */
public data class VerificationReviewHistoryPage(public val snapshots: List<VerificationReviewSnapshot>,
    public val nextCursor: String?) {
    override fun toString(): String = "VerificationReviewHistoryPage([PRIVATE])"
}

/** Optional read-only capability borrowing the original project's existing
 * owner. Native storage admits at most eight records and 2 MiB encoded bytes;
 * public DTOs cannot establish their original encoded size. Decoded notes remain
 * bounded to 1 MiB per page. All returned snapshots have a saved version > 0. */
public interface WorkbenchVerificationHistory {
    /** The opaque cursor is the last returned canonical store key, exclusive
     * on the next page. No cursor is returned when there are no further rows. */
    public suspend fun page(after: String? = null, limit: UInt = 8u): VerificationReviewHistoryPage
    /** Checks the exact saved snapshot and original store project identity.
     * Historical notes do not assert current document/source revision. */
    public suspend fun check(expected: VerificationReviewSnapshot): Unit
}

public expect fun verificationHistory(original: WorkbenchProject): WorkbenchVerificationHistory
