package com.visualworkbench.shared

import com.visualworkbench.bindings.core.Cancellation
import com.visualworkbench.bindings.core.PackageException
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.VerificationPackageKey as NKey
import com.visualworkbench.bindings.core.VerificationPair as NPair
import com.visualworkbench.bindings.core.VerificationVerdict as NVerdict
import com.visualworkbench.bindings.core.VerificationMarkerReview as NMarker
import com.visualworkbench.bindings.core.VerificationReviewSnapshot as NSnapshot
import com.visualworkbench.bindings.core.VerificationMarkerTranslation as NTranslation
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.sync.Semaphore

private val verificationScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
private val verificationSlots = Semaphore(2)

private fun verificationFailure(error: PackageException): PackageFailure = PackageFailure(when (error) {
    is PackageException.Invalid -> PackageFailureKind.Invalid
    is PackageException.Limit -> PackageFailureKind.Limit
    is PackageException.Stale -> PackageFailureKind.Stale
    is PackageException.Original -> PackageFailureKind.Original
    is PackageException.Depth -> PackageFailureKind.Depth
    is PackageException.Unsupported -> PackageFailureKind.Unsupported
    is PackageException.Integrity -> PackageFailureKind.Integrity
    is PackageException.Identity -> PackageFailureKind.Identity
    is PackageException.Storage -> PackageFailureKind.Storage
    is PackageException.Busy -> PackageFailureKind.Busy
    is PackageException.Closed -> PackageFailureKind.Closed
    is PackageException.Cancelled -> PackageFailureKind.Cancelled
})

private suspend fun <T> settledVerification(release: suspend (T) -> Unit = {},
    block: suspend (Cancellation) -> T): T = ownPackageResult(verificationScope, verificationSlots, ::Cancellation,
    { it.cancel() }, { it.destroy() }, release) { cancel ->
    try { block(cancel) }
    catch (error: PackageException) { throw verificationFailure(error) }
    catch (_: IllegalStateException) { throw PackageFailure(PackageFailureKind.Closed) }
}

private fun verificationProject(project: WorkbenchProject): ProjectSession =
    try { nativeProjectHandle(project) }
    catch (error: SessionFailure) {
        throw PackageFailure(if (error.kind == SessionFailureKind.Closed) PackageFailureKind.Closed else PackageFailureKind.Invalid)
    }

private fun VerificationPackageKey.native(): NKey = NKey(packageId, target, manifestSha256)
private fun NKey.common(): VerificationPackageKey = VerificationPackageKey(packageId, target, manifestSha256)
private fun VerificationPair.native(): NPair = NPair(original.native(), after.native(), receiptId, receiptBlake3)
private fun NPair.common(): VerificationPair = VerificationPair(original.common(), after.common(), receiptId, receiptBlake3)
private fun VerificationVerdict.native(): NVerdict = when (this) {
    VerificationVerdict.Pending -> NVerdict.PENDING
    VerificationVerdict.Pass -> NVerdict.PASS
    VerificationVerdict.Fail -> NVerdict.FAIL
}
private fun NVerdict.common(): VerificationVerdict = when (this) {
    NVerdict.PENDING -> VerificationVerdict.Pending
    NVerdict.PASS -> VerificationVerdict.Pass
    NVerdict.FAIL -> VerificationVerdict.Fail
}
private fun VerificationReviewSnapshot.native(): NSnapshot = NSnapshot(pair.native(), projectId, markerCount, version,
    markers.map { NMarker(it.originalNumber, it.verdict.native(), it.note) })
private fun NSnapshot.common(pair: VerificationPair): VerificationReviewSnapshot {
    if (markers.size > VERIFICATION_MAX_MARKERS) throw PackageFailure(PackageFailureKind.Integrity)
    return ownedVerificationSnapshot(VerificationReviewSnapshot(this.pair.common(), projectId, markerCount, version,
        markers.map { VerificationMarkerReview(it.originalNumber, it.verdict.common(), it.note) }), pair, PackageFailureKind.Integrity)
}

public actual fun verificationWorkflows(original: WorkbenchProject, after: WorkbenchProject,
    catalog: WorkbenchPackageCatalog, inbox: WorkbenchMcpInbox): WorkbenchVerification {
    // Admission checks borrow existing owners. Their real native methods also
    // check closure/currentness; this factory creates no independent lifetime.
    verificationProject(original)
    verificationProject(after)
    nativePackageCatalog(catalog)
    nativeMcpInbox(inbox)
    return NativeVerification(original, after, catalog, inbox)
}

private class NativeVerification(private val original: WorkbenchProject, private val after: WorkbenchProject,
    private val catalog: WorkbenchPackageCatalog, private val inbox: WorkbenchMcpInbox) : WorkbenchVerification {
    override suspend fun review(pair: VerificationPair): VerificationReviewSnapshot {
        validateVerificationPair(pair)
        return settledVerification { cancel ->
            verificationProject(original).verificationReview(verificationProject(after), nativePackageCatalog(catalog),
                nativeMcpInbox(inbox), pair.native(), cancel).common(pair)
        }
    }

    override suspend fun change(pair: VerificationPair, expectedVersion: ULong, operationId: String,
        originalNumber: UInt, verdict: VerificationVerdict, note: String): VerificationReviewSnapshot {
        validateVerificationPair(pair)
        validateVerificationChange(expectedVersion, operationId, originalNumber, note)
        if (expectedVersion == Long.MAX_VALUE.toULong()) throw PackageFailure(PackageFailureKind.Limit)
        return settledVerification { cancel ->
            val result = verificationProject(original).changeVerificationReview(verificationProject(after),
                nativePackageCatalog(catalog), nativeMcpInbox(inbox), pair.native(), expectedVersion, operationId,
                originalNumber, verdict.native(), note, cancel).common(pair)
            if (result.version != expectedVersion + 1uL || originalNumber > result.markerCount ||
                result.markers[originalNumber.toInt() - 1].let { it.verdict != verdict || it.note != note })
                throw PackageFailure(PackageFailureKind.Integrity)
            result
        }
    }

    override suspend fun compileFailures(expected: VerificationReviewSnapshot,
        translations: List<VerificationMarkerTranslation>, packageId: String, createdAtMs: Long,
        memoryBudgetBytes: ULong): WorkbenchCompiledPackage? {
        val review = ownedVerificationSnapshot(expected)
        val measured = ownedVerificationTranslations(review, translations)
        if (!verificationId(packageId) || packageId == review.pair.original.packageId ||
            packageId == review.pair.after.packageId || createdAtMs !in 0L..253_402_300_799_999L)
            throw PackageFailure(PackageFailureKind.Invalid)
        if (memoryBudgetBytes !in 16uL * 1024uL * 1024uL..256uL * 1024uL * 1024uL)
            throw PackageFailure(PackageFailureKind.Limit)
        val failed = review.markers.count { it.verdict == VerificationVerdict.Fail }.toUInt()
        return settledVerification<WorkbenchCompiledPackage?>(release = { it?.close() }) { cancel ->
            val handle = verificationProject(original).compileVerificationFailures(verificationProject(after),
                nativePackageCatalog(catalog), nativeMcpInbox(inbox), review.pair.native(), review.native(),
                measured.map { NTranslation(it.originalNumber, it.dx, it.dy) }, packageId, createdAtMs, memoryBudgetBytes, cancel)
            val compiled = handle?.let(::ownNativeCompiled)
            try {
                if ((failed == 0u) != (compiled == null)) throw PackageFailure(PackageFailureKind.Integrity)
                compiled?.info?.let { info ->
                    if (info.packageId != packageId || info.target != review.pair.original.target || info.markerCount != failed)
                        throw PackageFailure(PackageFailureKind.Integrity)
                }
                compiled
            } catch (error: Throwable) {
                try { compiled?.close() } catch (cleanup: Throwable) { error.addSuppressed(cleanup) }
                throw error
            }
        }
    }
}

/* ROOT-OWNED INTEGRATION (not applied by this leaf)
 * NativePackages.kt, immediately after NativeCompiled:
 *
 * internal fun ownNativeCompiled(handle:NCompiled):WorkbenchCompiledPackage {
 *     try { return NativeCompiled(handle) }
 *     catch(error:Throwable) {
 *         try { handle.dispose() } catch(cleanup:Throwable) { error.addSuppressed(cleanup) }
 *         finally { try { handle.destroy() } catch(cleanup:Throwable) { error.addSuppressed(cleanup) } }
 *         throw error
 *     }
 * }
 *
 * NativeMcpInbox.kt:
 * -private class NativeInbox(private val handle:NInbox):WorkbenchMcpInbox {
 * +private class NativeInbox(val handle:NInbox):WorkbenchMcpInbox {
 *      private val closer=JoinedPackageClose()
 * +    fun check(){closer.check()}
 *
 * Append:
 * internal fun nativeMcpInbox(value:WorkbenchMcpInbox):NInbox {
 *     val inbox=value as? NativeInbox?:throw PackageFailure(PackageFailureKind.Invalid)
 *     inbox.check();return inbox.handle
 * }
 *
 * Existing NativeCompiled/JoinedPackageClose owns every normal or late result.
 * Core binding generation and aggregate compilation remain root-owned.
 */
