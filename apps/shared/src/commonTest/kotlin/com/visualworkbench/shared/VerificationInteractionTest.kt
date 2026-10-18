package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertSame
import kotlin.test.assertTrue

class VerificationInteractionTest {
    private val original = VerificationPackageKey("0199a000-0000-7000-8000-000000000001", "generic", "a".repeat(64))
    private val after = VerificationPackageKey("0199a000-0000-7000-8000-000000000002", "generic", "b".repeat(64))
    private val pair = VerificationPair(original, after, "0199a000-0000-7000-8000-000000000003", "c".repeat(64))
    private val projectId = "0199a000-0000-7000-8000-000000000004"
    private val operationId = "0199a000-0000-7000-8000-000000000005"

    private fun snapshot(vararg verdicts: VerificationVerdict, version: ULong = 1uL): VerificationReviewSnapshot =
        VerificationReviewSnapshot(pair, projectId, verdicts.size.toUInt(), version,
            verdicts.mapIndexed { index, verdict -> VerificationMarkerReview((index + 1).toUInt(), verdict, "") })

    private fun failure(kind: PackageFailureKind, block: () -> Unit): Unit {
        assertEquals(kind, assertFailsWith<PackageFailure>(block = block).kind)
    }

    private fun historySnapshot(number: Int = 1): VerificationReviewSnapshot = snapshot(VerificationVerdict.Pending)
        .copy(pair = pair.copy(receiptId = pair.receiptId.dropLast(1) + number.toString(16)))

    @Test fun freshVersionZeroCanSaveItsFirstManualDecisionAsVersionOne(): Unit {
        val interaction = VerificationInteraction()
        val fresh = snapshot(VerificationVerdict.Pending, VerificationVerdict.Pending, version = 0uL)
        interaction.loaded(pair, fresh)
        assertEquals(fresh, interaction.snapshot)
        assertSame(VerificationReviewOutcome.AwaitingOwner, interaction.outcome())
        val first = interaction.ownerChange(operationId, 1u, VerificationVerdict.Fail, "owner first decision")
        assertEquals(0uL, first.expectedVersion)
        failure(PackageFailureKind.Integrity) { interaction.changed(first, fresh) }
        assertEquals(first, interaction.pending)
        val saved = fresh.copy(version = 1uL, markers = listOf(
            VerificationMarkerReview(1u, VerificationVerdict.Fail, first.note), fresh.markers[1]))
        interaction.changed(first, saved)
        assertNull(interaction.pending)
        assertEquals(saved, interaction.snapshot)
        assertSame(VerificationReviewOutcome.AwaitingOwner, interaction.outcome())
    }

    @Test fun versionZeroRefusesSavedDecisionsAndNotesButAdmitsAnEmptyFreshReview(): Unit {
        val interaction = VerificationInteraction()
        val fresh = snapshot(VerificationVerdict.Pending, version = 0uL)
        for (row in listOf(VerificationMarkerReview(1u, VerificationVerdict.Pass, ""),
            VerificationMarkerReview(1u, VerificationVerdict.Fail, ""),
            VerificationMarkerReview(1u, VerificationVerdict.Pending, "owner note"))) {
            failure(PackageFailureKind.Integrity) { interaction.loaded(pair, fresh.copy(markers = listOf(row))) }
        }
        assertNull(interaction.snapshot)
        val empty = snapshot(version = 0uL)
        interaction.loaded(pair, empty)
        assertEquals(empty, interaction.snapshot)
        assertSame(VerificationReviewOutcome.AllPass, interaction.outcome())
    }

    @Test fun failedSubsetKeepsOriginalNumbersAndPendingNeverLooksComplete(): Unit {
        val pending = snapshot(VerificationVerdict.Pass, VerificationVerdict.Pending, VerificationVerdict.Fail)
        assertSame(VerificationReviewOutcome.AwaitingOwner, verificationReviewOutcome(pending))
        val complete = snapshot(VerificationVerdict.Pass, VerificationVerdict.Pass, VerificationVerdict.Fail)
        assertEquals(VerificationReviewOutcome.Failures(listOf(3u)), verificationReviewOutcome(complete))
        assertSame(VerificationReviewOutcome.AllPass,
            verificationReviewOutcome(snapshot(VerificationVerdict.Pass, VerificationVerdict.Pass)))
    }

    @Test fun uncertainSaveRetainsExactRequestAndDoesNotAcceptAnUnrelatedReply(): Unit {
        val interaction = VerificationInteraction()
        val before = snapshot(VerificationVerdict.Pending, VerificationVerdict.Pass)
        interaction.loaded(pair, before)
        val request = interaction.ownerChange(operationId, 1u, VerificationVerdict.Fail, "owner-private-note")
        failure(PackageFailureKind.Busy) { interaction.ownerChange(operationId, 2u, VerificationVerdict.Pass, "") }
        val receipt = before.copy(version = 2uL, markers = listOf(
            VerificationMarkerReview(1u, VerificationVerdict.Fail, "owner-private-note"), before.markers[1]))
        failure(PackageFailureKind.Integrity) {
            interaction.changed(request, receipt.copy(pair = pair.copy(receiptBlake3 = "d".repeat(64))))
        }
        assertEquals(request, interaction.pending)
        assertEquals(before, interaction.snapshot)
        interaction.changed(request, receipt)
        assertNull(interaction.pending)
        assertEquals(VerificationReviewOutcome.Failures(listOf(1u)), interaction.outcome())
    }

    @Test fun aReplyCannotSilentlyOverwriteAnotherMarkerOrJumpVersions(): Unit {
        val interaction = VerificationInteraction()
        val before = snapshot(VerificationVerdict.Pass, VerificationVerdict.Pending)
        interaction.loaded(pair, before)
        val request = interaction.ownerChange(operationId, 2u, VerificationVerdict.Pass, "")
        val changed = snapshot(VerificationVerdict.Fail, VerificationVerdict.Pass, version = 2uL)
        failure(PackageFailureKind.Integrity) { interaction.changed(request, changed) }
        failure(PackageFailureKind.Integrity) {
            interaction.changed(request, snapshot(VerificationVerdict.Pass, VerificationVerdict.Pass, version = 3uL))
        }
        assertEquals(request, interaction.pending)
        interaction.loaded(pair, snapshot(VerificationVerdict.Pass, VerificationVerdict.Pass, version = 3uL))
        assertNull(interaction.pending)
        assertSame(VerificationReviewOutcome.AllPass, interaction.outcome())
    }

    @Test fun retainedSnapshotDoesNotAliasACallersMutableList(): Unit {
        val rows = mutableListOf(VerificationMarkerReview(1u, VerificationVerdict.Pass, ""))
        val interaction = VerificationInteraction()
        interaction.loaded(pair, VerificationReviewSnapshot(pair, projectId, 1u, 1uL, rows))
        rows[0] = VerificationMarkerReview(1u, VerificationVerdict.Fail, "changed later")
        assertEquals(VerificationVerdict.Pass, interaction.snapshot?.markers?.single()?.verdict)
        assertSame(VerificationReviewOutcome.AllPass, interaction.outcome())
    }

    @Test fun notesArePrivateAndLimitedByUtf8BytesAcrossTheReview(): Unit {
        val secret = "private owner observation"
        val row = VerificationMarkerReview(1u, VerificationVerdict.Fail, secret)
        val value = VerificationReviewSnapshot(pair, projectId, 1u, 1uL, listOf(row))
        val interaction = VerificationInteraction()
        interaction.loaded(pair, value)
        val request = interaction.ownerChange(operationId, 1u, VerificationVerdict.Pass, secret)
        for (text in listOf(row.toString(), value.toString(), request.toString())) assertFalse(secret in text)
        interaction.loaded(pair, value)
        failure(PackageFailureKind.Limit) { interaction.ownerChange(operationId, 1u, VerificationVerdict.Pass, "\u20ac".repeat(10923)) }
        failure(PackageFailureKind.Invalid) { interaction.ownerChange(operationId, 1u, VerificationVerdict.Pass, "a\u0000b") }
        failure(PackageFailureKind.Invalid) { interaction.ownerChange(operationId, 1u, VerificationVerdict.Pass, "\uD800") }
        val full = snapshot(*Array(5) { VerificationVerdict.Pass }).copy(markers = (1u..5u).map {
            VerificationMarkerReview(it, VerificationVerdict.Pass, if (it <= 4u) "x".repeat(32768) else "")
        })
        interaction.loaded(pair, full)
        failure(PackageFailureKind.Limit) { interaction.ownerChange(operationId, 5u, VerificationVerdict.Fail, "x") }
        assertNull(interaction.pending)
    }

    @Test fun malformedStoredShapesCannotEnterTheOwnerSurface(): Unit {
        val interaction = VerificationInteraction()
        val valid = snapshot(VerificationVerdict.Pass)
        for (bad in listOf(valid.copy(version = ULong.MAX_VALUE),
            valid.copy(markerCount = 2u), valid.copy(markers = listOf(valid.markers[0].copy(originalNumber = 0u))),
            valid.copy(projectId = projectId.uppercase()),
            valid.copy(pair = pair.copy(original = original.copy(target = "arbitrary"))))) {
            failure(PackageFailureKind.Integrity) { interaction.loaded(bad.pair, bad) }
        }
        assertNull(interaction.snapshot)
        val huge = valid.copy(markerCount = 513u, markers = List(513) {
            VerificationMarkerReview((it + 1).toUInt(), VerificationVerdict.Pending, "")
        })
        failure(PackageFailureKind.Integrity) { interaction.loaded(pair, huge) }
    }

    @Test fun receiptIdentityMatchesTheExistingNativeInboxUuidV7Contract(): Unit {
        val receiptPair = pair.copy(receiptId = "0199a000-0000-4000-8000-000000000003")
        val interaction = VerificationInteraction()
        failure(PackageFailureKind.Integrity) {
            interaction.loaded(receiptPair, snapshot(VerificationVerdict.Pass).copy(pair = receiptPair))
        }
        interaction.loaded(pair, snapshot(VerificationVerdict.Pass))
        assertEquals(pair, interaction.snapshot?.pair)
    }

    @Test fun compileProjectionRequiresEachFailedNumberExactlyOnceAndMeasuredFiniteOffsets(): Unit {
        val complete = snapshot(VerificationVerdict.Fail, VerificationVerdict.Pass, VerificationVerdict.Fail)
        val translations = mutableListOf(VerificationMarkerTranslation(3u, 4.0, -2.0), VerificationMarkerTranslation(1u, 1.0, 2.0))
        val owned = ownedVerificationTranslations(complete, translations)
        assertEquals(listOf(1u, 3u), owned.map { it.originalNumber })
        translations.clear()
        assertEquals(2, owned.size)
        for (bad in listOf(emptyList(), listOf(VerificationMarkerTranslation(1u, 0.0, 0.0), VerificationMarkerTranslation(1u, 0.0, 0.0)),
            listOf(VerificationMarkerTranslation(1u, Double.NaN, 0.0), VerificationMarkerTranslation(3u, 0.0, 0.0)),
            listOf(VerificationMarkerTranslation(1u, 0.0, 0.0), VerificationMarkerTranslation(2u, 0.0, 0.0)))) {
            failure(PackageFailureKind.Invalid) { ownedVerificationTranslations(complete, bad) }
        }
        failure(PackageFailureKind.Invalid) {
            ownedVerificationTranslations(snapshot(VerificationVerdict.Pending), emptyList())
        }
        assertTrue(ownedVerificationTranslations(snapshot(VerificationVerdict.Pass), emptyList()).isEmpty())
    }

    @Test fun privateHistoryUsesCanonicalExclusiveCursorsAndBoundedRequests(): Unit {
        val after = VERIFICATION_HISTORY_PREFIX + "1".repeat(64)
        val next = VERIFICATION_HISTORY_PREFIX + "2".repeat(64)
        validateVerificationHistoryRequest(after, 1u)
        val page = ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(historySnapshot()), next), projectId, after, 1u)
        assertEquals(next, page.nextCursor)
        assertNull(ownedVerificationHistoryPage(VerificationReviewHistoryPage(emptyList(), null), projectId, next).nextCursor)
        for (cursor in listOf("", "1".repeat(64), next.uppercase(), VERIFICATION_HISTORY_PREFIX + "g".repeat(64),
            "vw.verification-review.v2." + "1".repeat(64), next + "0")) {
            failure(PackageFailureKind.Invalid) { validateVerificationHistoryRequest(cursor, 8u) }
            failure(PackageFailureKind.Integrity) {
                ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(historySnapshot()), cursor), projectId)
            }
        }
        for (limit in listOf(0u, 9u, UInt.MAX_VALUE)) {
            failure(PackageFailureKind.Invalid) { validateVerificationHistoryRequest(null, limit) }
        }
        for (cursor in listOf(after, VERIFICATION_HISTORY_PREFIX + "0".repeat(64))) {
            failure(PackageFailureKind.Integrity) {
                ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(historySnapshot()), cursor), projectId, after)
            }
        }
        failure(PackageFailureKind.Integrity) {
            ownedVerificationHistoryPage(VerificationReviewHistoryPage(emptyList(), next), projectId)
        }
    }

    @Test fun historyAdmitsSavedPendingNotesAndRefusesFreshOrForeignRecords(): Unit {
        val saved = historySnapshot().copy(markers = listOf(VerificationMarkerReview(1u, VerificationVerdict.Pending, "owner note")))
        assertEquals(saved, ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(saved), null), projectId).snapshots.single())
        for (bad in listOf(saved.copy(version = 0uL, markers = listOf(VerificationMarkerReview(1u, VerificationVerdict.Pending, ""))),
            saved.copy(version = ULong.MAX_VALUE), saved.copy(projectId = operationId),
            saved.copy(markers = listOf(VerificationMarkerReview(2u, VerificationVerdict.Pending, ""))))) {
            failure(PackageFailureKind.Integrity) {
                ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(bad), null), projectId)
            }
        }
        failure(PackageFailureKind.Integrity) {
            ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(saved, saved), null), projectId)
        }
    }

    @Test fun historyOwnsBothListsAndKeepsPrivateNotesOutOfDiagnostics(): Unit {
        val secret = "private historical owner observation"
        val markers = mutableListOf(VerificationMarkerReview(1u, VerificationVerdict.Fail, secret))
        val rows = mutableListOf(historySnapshot().copy(markers = markers))
        val owned = ownedVerificationHistoryPage(VerificationReviewHistoryPage(rows, null), projectId)
        markers[0] = VerificationMarkerReview(1u, VerificationVerdict.Pass, "changed later")
        rows.clear()
        assertEquals(1, owned.snapshots.size)
        assertEquals(VerificationVerdict.Fail, owned.snapshots.single().markers.single().verdict)
        assertEquals(secret, owned.snapshots.single().markers.single().note)
        assertFalse(secret in owned.toString())
        assertFalse(secret in owned.snapshots.toString())
    }

    @Test fun historyBoundsDecodedNotesWithoutClaimingToMeasureNativeEncodedRows(): Unit {
        val maxNotes = List(4) { index -> VerificationMarkerReview((index + 1).toUInt(), VerificationVerdict.Pass, "x".repeat(32768)) }
        val rows = (1..8).map { historySnapshot(it).copy(markerCount = 4u, markers = maxNotes) }
        val owned = ownedVerificationHistoryPage(VerificationReviewHistoryPage(rows, null), projectId)
        assertEquals(8, owned.snapshots.size)
        assertEquals(1024 * 1024, owned.snapshots.sumOf { row -> row.markers.sumOf { it.note.encodeToByteArray().size } })
        failure(PackageFailureKind.Integrity) {
            ownedVerificationHistoryPage(VerificationReviewHistoryPage(rows + historySnapshot(9), null), projectId)
        }
        failure(PackageFailureKind.Integrity) {
            ownedVerificationHistoryPage(VerificationReviewHistoryPage(rows.take(2), null), projectId, limit = 1u)
        }
        failure(PackageFailureKind.Limit) {
            val tooManyNotes = historySnapshot().copy(markerCount = 5u, markers = maxNotes + VerificationMarkerReview(5u, VerificationVerdict.Pass, "x"))
            ownedVerificationHistoryPage(VerificationReviewHistoryPage(listOf(tooManyNotes), null), projectId)
        }
    }
}
