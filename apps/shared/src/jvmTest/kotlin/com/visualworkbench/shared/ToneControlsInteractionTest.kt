package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

class ToneControlsInteractionTest {
    @Test fun capabilityRemainsLazyUntilPanelActivation() = runBlocking {
        val f = Fixture()
        try {
            f.publish(); repeat(3) { yield() }
            assertEquals(0,f.capabilities)
            f.controls.active(true); f.idle()
            assertEquals(1,f.reads); assertTrue(f.controls.state.value.ready)
            assertFalse(f.controls.state.value.previewAssumeSrgb)
        } finally { f.close() }
    }

    @Test fun invalidPrivateValuesNeverAllocateIdsOrPrepareCanonicalEdits() = runBlocking {
        val f = Fixture()
        try {
            f.open()
            val defaults = f.form()
            val invalid = listOf(defaults.copy(brightness="NaN"),defaults.copy(contrast="Infinity"),
                defaults.copy(brightness="1.01"),defaults.copy(inputBlack="1"),defaults.copy(inputWhite="0"),
                defaults.copy(gamma="0"),defaults.copy(gamma="-1"),defaults.copy(outputBlack="0.8",outputWhite="0.2"),
                defaults.copy(opacity="1.1"),defaults.copy(gamma=""),defaults.copy(inputWhite="1e999"))
            for (value in invalid) {
                assertTrue(f.controls.edit(f.token(),value))
                assertFalse(f.controls.apply(f.token()))
                assertFalse(f.controls.preview(f.token(),0u,0u,false))
            }
            assertFalse(f.controls.edit(f.token(),defaults.copy(gamma="1".repeat(33))))
            assertEquals(0,f.ids); assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares); assertEquals(0,f.previews)
            assertEquals("1e999",f.form().inputWhite)
        } finally { f.close() }
    }

    @Test fun dirtyDraftNeedsApplyOrExplicitDiscardBeforeSelectionOrClosing() = runBlocking {
        val f = Fixture()
        try {
            f.open(); assertTrue(f.controls.edit(f.token(),f.form().copy(brightness="0.4")))
            assertTrue(f.controls.state.value.dirty); assertFalse(f.controls.state.value.mayLeave)
            assertFalse(f.controls.active(false)); assertFalse(f.controls.selectTone(f.token(),id(8)))
            assertFalse(f.controls.remove(f.token())); assertEquals(0,f.prepares)
            assertTrue(f.controls.discardFormAndReload(f.token())); f.idle()
            assertFalse(f.controls.state.value.dirty); assertTrue(f.controls.active(false))
        } finally { f.close() }
    }

    @Test fun previewDraftUsesStableIdsAndApplyMakesOnePlanForThoseExactParameters() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.controls.edit(f.token(),f.form().copy(brightness="0.25",gamma="1.2",opacity="0.75"))
            assertTrue(f.controls.preview(f.token(),0u,0u,false)); f.previewIdle()
            val first = f.previewCommands.single() as ToneControlCommand.Set
            assertTrue(f.controls.preview(f.token(),1u,2u,false)); f.previewIdle()
            assertEquals(first,f.previewCommands.last())
            assertEquals(2,f.ids); assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares)
            assertEquals(0,f.commits); assertTrue(f.controls.state.value.dirty)
            f.mutations.lock()
            assertTrue(f.controls.apply(f.token())); assertFalse(f.controls.apply(f.token()))
            f.mutations.unlock(); f.idle()
            assertEquals(first,f.commands.single()); assertEquals(1,f.metadataCalls); assertEquals(1,f.prepares)
            assertEquals(1,f.commits); assertEquals(1,f.closedPlans); assertNotNull(f.controls.state.value.lastReceipt)
        } finally { if (f.mutations.isLocked) f.mutations.unlock(); f.close() }
    }

    @Test fun oneConflatedReaderKeepsOnlyLatestQueuedPreviewAndRejectsOldDraftPixels() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.open()
            f.previewGate = { number -> if (number == 1) { entered.complete(Unit); release.await() } }
            assertTrue(f.controls.preview(f.token(),0u,0u,false)); entered.await()
            f.controls.edit(f.token(),f.form().copy(gamma="2.0"))
            for (x in 1u..20u) assertTrue(f.controls.preview(f.token(),x,3u,true))
            assertEquals(1,f.previews); assertNull(f.controls.state.value.preview)
            release.complete(Unit); f.previewIdle()
            assertEquals(2,f.previews); assertEquals(20u,f.controls.state.value.preview!!.x)
            assertEquals(2.0,(f.previewCommands.last() as ToneControlCommand.Set).settings.gamma,0.0)
            assertEquals(listOf(false,true),f.previewConsents)
            assertEquals(0,f.prepares)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun localPreviewInputChangesAndFormChangesInvalidateOutstandingPixels() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.open(); f.previewGate = { entered.complete(Unit); release.await() }
            f.controls.preview(f.token(),0u,0u,false); entered.await()
            f.controls.invalidatePreview(f.token())
            release.complete(Unit); repeat(5) { yield() }
            assertNull(f.controls.state.value.preview); assertFalse(f.controls.state.value.previewLoading)
            f.previewGate = null
            f.controls.preview(f.token(),0u,0u,false); f.previewIdle()
            assertNotNull(f.controls.state.value.preview)
            f.controls.edit(f.token(),f.form().copy(contrast="0.2"))
            assertNull(f.controls.state.value.preview)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun savedPreviewDoesNotValidateOrCommitInvalidPrivateTyping() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.controls.edit(f.token(),f.form().copy(gamma="-"))
            assertTrue(f.controls.preview(f.token(),0u,0u,false,draft=false)); f.previewIdle()
            assertNull(f.previewCommands.single()); assertEquals(0,f.ids); assertEquals(0,f.metadataCalls)
            assertTrue(f.controls.state.value.dirty); assertFalse(f.controls.state.value.previewDraft)
        } finally { f.close() }
    }

    @Test fun dirtyDraftStaysStaleEvenIfAnObservedChangeReturnsToTheSameHash() = runBlocking {
        val f = Fixture()
        try {
            f.open(); val old = f.token()
            f.controls.edit(old,f.form().copy(brightness="0.3")); f.controls.observe(1uL)
            assertNull(f.controls.capture()); f.publish(1uL); f.idle()
            assertTrue(f.controls.state.value.stale); assertTrue(f.controls.state.value.dirty)
            assertEquals("0.3",f.form().brightness)
            assertFalse(f.controls.apply(old)); assertFalse(f.controls.apply(f.token()))
            assertTrue(f.controls.discardFormAndReload(f.token())); f.idle()
            assertFalse(f.controls.state.value.stale); assertFalse(f.controls.state.value.dirty)
            assertEquals(0,f.prepares)
        } finally { f.close() }
    }

    @Test fun delayedSnapshotCannotReintroduceOldMetadataAfterOps() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.readGate = { entered.complete(Unit); release.await() }
            f.publish(); f.controls.active(true); entered.await()
            f.controls.observe(1uL); release.complete(Unit); repeat(3) { yield() }
            assertNull(f.controls.state.value.snapshot)
            f.readGate = null; f.info = f.info.copy(stateHash="b".repeat(64)); f.publish(1uL); f.idle()
            assertEquals(f.info.stateHash,f.controls.state.value.snapshot!!.binding.stateHash)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun selectingAToneFencesAllCallbacksCapturedForThePreviousSelection() = runBlocking {
        val f = Fixture()
        try {
            f.open(); val previous = f.token()
            assertTrue(f.controls.selectTone(previous,id(8)))
            assertFalse(f.controls.selectTone(previous,null))
            assertFalse(f.controls.edit(previous,ToneControlForm(brightness="0.5")))
            assertFalse(f.controls.apply(previous)); assertFalse(f.controls.remove(previous))
            assertFalse(f.controls.preview(previous,0u,0u,false)); assertFalse(f.controls.discardFormAndReload(previous))
            assertEquals(id(8),f.form().objectId); assertEquals(0,f.prepares); assertEquals(0,f.previews)
        } finally { f.close() }
    }

    @Test fun identicalRevisionOnANewPageOrAttachmentRejectsOldPanelCallbacks() = runBlocking {
        val f = Fixture()
        try {
            f.open(); val old = f.token()
            f.controls.bind(f.project,f.document().copy(page=1u),0,0uL); f.idle()
            assertEquals(1u,f.token().page); assertFalse(f.controls.selectTone(old,id(8)))
            assertFalse(f.controls.apply(old)); assertFalse(f.controls.preview(old,0u,0u,false))
            val page = f.token()
            f.controls.bind(f.project,f.document(),1,0uL); f.idle()
            assertFalse(f.controls.edit(page,f.form().copy(brightness="0.5")))
            assertFalse(f.controls.apply(page)); assertEquals(0,f.prepares)
        } finally { f.close() }
    }

    @Test fun closingAndReopeningACleanPanelFencesOldCallbacks() = runBlocking {
        val f = Fixture()
        try {
            f.open(); val old = f.token()
            assertTrue(f.controls.active(false)); assertTrue(f.controls.active(true)); f.idle()
            assertFalse(f.controls.apply(old)); assertFalse(f.controls.selectTone(old,id(8)))
            assertFalse(f.controls.preview(old,0u,0u,false)); assertEquals(0,f.prepares)
        } finally { f.close() }
    }

    @Test fun metadataSuspensionCannotPrepareAfterVisibleBindingChanges() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.open(); f.metadataGate = { entered.complete(Unit); release.await() }
            assertTrue(f.controls.apply(f.token())); entered.await()
            f.info = f.info.copy(stateHash="c".repeat(64)); f.publish(1uL)
            release.complete(Unit); f.idle()
            assertEquals(0,f.prepares); assertEquals(0,f.commits)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun uncertainCommitRetainsExactlyOnePlanAndRetryUsesItsOriginalTransaction() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.failFirstCommit = true
            assertTrue(f.controls.apply(f.token())); f.idle()
            val retained = f.controls.state.value.retry!!
            assertFalse(f.controls.apply(f.token())); assertFalse(f.controls.state.value.mayLeave)
            assertTrue(f.controls.retry(f.token())); f.idle()
            assertEquals(2,f.ids); assertEquals(1,f.metadataCalls); assertEquals(1,f.prepares)
            assertEquals(2,f.commits); assertEquals(1,f.closedPlans)
            assertEquals(retained.transactionId,f.controls.state.value.lastReceipt!!.transactionId)
            assertTrue(f.controls.state.value.lastReceipt!!.duplicate)
        } finally { f.close() }
    }

    @Test fun operationCancellationLeavesTheExactRetryAndDoesNotCancelThePanelScope() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.commitGate = { if (f.commits == 1) throw CancellationException("operation cancelled") }
            assertTrue(f.controls.apply(f.token())); f.idle()
            assertNotNull(f.controls.state.value.retry); assertNotNull(f.controls.capture())
            assertTrue(f.controls.retry(f.token())); f.idle()
            assertEquals(1,f.prepares); assertEquals(1,f.metadataCalls); assertEquals(2,f.commits)
        } finally { f.close() }
    }

    @Test fun acceptedReceiptIsRecordedBeforeFailedRootRefresh() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.controls.edit(f.token(),f.form().copy(contrast="0.2")); f.failRefresh = true
            assertTrue(f.controls.apply(f.token())); f.idle()
            assertNotNull(f.controls.state.value.lastReceipt); assertNull(f.controls.state.value.retry)
            assertFalse(f.controls.state.value.dirty); assertEquals(1,f.closedPlans); assertTrue(f.controls.state.value.mayLeave)
        } finally { f.close() }
    }

    @Test fun existingToneUpdatesAreOneAtomicCommandWithOriginalIdsAndNoNewIdentity() = runBlocking {
        val f = Fixture()
        try {
            f.open(); assertTrue(f.controls.selectTone(f.token(),id(8)))
            f.controls.edit(f.token(),f.form().copy(gamma="2.0",enabled=false,opacity="0.4"))
            assertTrue(f.controls.apply(f.token())); f.idle()
            val value = f.commands.single() as ToneControlCommand.Set
            assertEquals(id(7),value.layerId); assertEquals(id(8),value.objectId)
            assertFalse(value.enabled); assertEquals(0.4,value.opacity,0.0); assertEquals(2.0,value.settings.gamma,0.0)
            assertEquals(0,f.ids); assertEquals(1,f.prepares); assertEquals(1,f.commits)
        } finally { f.close() }
    }

    @Test fun explicitRemoveUsesOnlyTheSelectedSavedToneAndOnePlan() = runBlocking {
        val f = Fixture()
        try {
            f.open(); assertFalse(f.controls.remove(f.token()))
            assertTrue(f.controls.selectTone(f.token(),id(8)))
            assertTrue(f.controls.remove(f.token())); f.idle()
            assertEquals(ToneControlCommand.Remove(id(7),id(8)),f.commands.single())
            assertEquals(0,f.ids); assertEquals(1,f.prepares); assertEquals(1,f.commits)
        } finally { f.close() }
    }

    @Test fun detachJoinsActualPreviewAndCommitSettlementBeforeClosingThePlan() = runBlocking {
        val f = Fixture()
        val reading = CompletableDeferred<Unit>(); val releaseRead = CompletableDeferred<Unit>()
        val writing = CompletableDeferred<Unit>(); val releaseWrite = CompletableDeferred<Unit>()
        var detaching: Deferred<Unit>? = null
        try {
            f.open()
            f.previewGate = { withContext(NonCancellable) { reading.complete(Unit); releaseRead.await() }; currentCoroutineContext().ensureActive() }
            f.commitGate = { withContext(NonCancellable) { writing.complete(Unit); releaseWrite.await() }; currentCoroutineContext().ensureActive() }
            f.controls.preview(f.token(),0u,0u,false); reading.await()
            f.controls.apply(f.token()); writing.await()
            f.controls.cancel(f.token())
            assertTrue(f.controls.state.value.busy); assertFalse(f.controls.apply(f.token())); assertFalse(f.controls.active(false))
            detaching = async(start=CoroutineStart.UNDISPATCHED) { f.controls.detach() }
            assertFalse(detaching.isCompleted); assertEquals(0,f.closedPlans)
            releaseRead.complete(Unit); repeat(3) { yield() }; assertFalse(detaching.isCompleted)
            releaseWrite.complete(Unit); withTimeout(3000) { detaching.await() }
            assertEquals(1,f.previewFinished); assertEquals(1,f.closedPlans)
            assertNull(f.controls.state.value.retry); assertFalse(f.controls.state.value.busy)
        } finally { releaseRead.complete(Unit); releaseWrite.complete(Unit); withContext(NonCancellable) { detaching?.join(); f.close() } }
    }

    @Test fun lateAcceptedOldAttachmentCannotRefreshOrPublishIntoItsReplacement() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.open(); f.commitGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
            f.controls.apply(f.token()); entered.await()
            f.controls.bind(f.project,f.document(),1,0uL)
            release.complete(Unit); f.idle()
            assertNull(f.controls.state.value.lastReceipt); assertEquals(0,f.refreshCalls)
            assertEquals(1,f.commits); assertEquals(1,f.closedPlans)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun cancelledParentRejectsLateCallbacksAndCloseStillJoinsItsReaders() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var closing: Deferred<Unit>? = null
        try {
            f.open(); val token = f.token()
            f.previewGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
            f.controls.preview(token,0u,0u,false); entered.await()
            f.scope.cancel(); val calls = f.capabilities
            assertNull(f.controls.capture()); assertFalse(f.controls.active(true)); assertFalse(f.controls.apply(token))
            assertFalse(f.controls.selectTone(token,id(8))); assertFalse(f.controls.preview(token,0u,0u,false))
            closing = async(start=CoroutineStart.UNDISPATCHED) { f.controls.close() }
            assertFalse(closing.isCompleted)
            release.complete(Unit); withTimeout(3000) { closing.await() }
            assertEquals(calls,f.capabilities); assertEquals(0,f.prepares); assertNull(f.controls.state.value.preview)
        } finally { release.complete(Unit); withContext(NonCancellable) { closing?.join(); f.close() } }
    }

    @Test fun operationLevelSnapshotCancellationCanBeReloadedOnTheSameReader() = runBlocking {
        val f = Fixture()
        try {
            f.readFailure = CancellationException("operation cancelled")
            f.publish(); f.controls.active(true); f.idle()
            assertNull(f.controls.state.value.snapshot); assertNotNull(f.controls.state.value.issue)
            f.readFailure = null; assertTrue(f.controls.discardFormAndReload(f.token())); f.idle()
            assertTrue(f.controls.state.value.ready); assertEquals(2,f.reads)
        } finally { f.close() }
    }

    @Test fun repeatedCloseCallsBothWaitForTheActualReader() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var first: Deferred<Unit>? = null; var second: Deferred<Unit>? = null
        try {
            f.open()
            f.previewGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
            f.controls.preview(f.token(),0u,0u,false); entered.await()
            first = async(start=CoroutineStart.UNDISPATCHED) { f.controls.close() }
            second = async(start=CoroutineStart.UNDISPATCHED) { f.controls.close() }
            assertFalse(first.isCompleted); assertFalse(second.isCompleted)
            release.complete(Unit); withTimeout(3000) { first.await(); second.await() }
            assertEquals(1,f.previewFinished); assertNull(f.controls.state.value.preview)
        } finally { release.complete(Unit); withContext(NonCancellable) { first?.join(); second?.join(); f.close() } }
    }

    @Test fun previewOwnsInjectedPixelsAndRefusesMismatchedSourceRevisionOrRegion() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.controls.preview(f.token(),0u,0u,false); f.previewIdle()
            val accepted = f.controls.state.value.preview!!
            val raw = f.lastRawPreview!!
            raw.rgba.fill(99)
            assertEquals(1.toByte(),accepted.rgba[0])
            assertFalse(accepted.toString().contains("rgba=[")); assertTrue(accepted.toString().contains("rgbaBytes="))
            val corruptions: List<(ToneControlPreview) -> ToneControlPreview> = listOf(
                { it.copy(sourceAssetId="e".repeat(64)) },{ it.copy(binding=it.binding.copy(hostSeq=2uL)) },
                { it.copy(x=1u) },{ it.copy(rgba=ByteArray(4)) })
            for (corrupt in corruptions) {
                f.corruptPreview = corrupt
                f.controls.preview(f.token(),0u,0u,false); f.previewIdle()
                assertNull(f.controls.state.value.preview); assertNotNull(f.controls.state.value.issue)
            }
            assertFalse(f.controls.preview(f.token(),SelectionRegion(UInt.MAX_VALUE,0u,2u,1u),false))
            assertFalse(f.controls.preview(f.token(),SelectionRegion(0u,0u,513u,1u),false))
        } finally { f.close() }
    }

    @Test fun toneCatalogIsOwnedAndDuplicateOrLockedTargetsCannotRedirectAnEdit() = runBlocking {
        val f = Fixture()
        try {
            val entries = f.tones.toMutableList(); f.tones = entries
            f.open(); entries.clear(); assertEquals(1,f.controls.state.value.snapshot!!.tones.size)
            f.tones = listOf(ToneControlEntry(id(7),id(8),true,true,1.0,ImageToneSettings()))
            f.controls.discardFormAndReload(f.token()); f.idle(); f.controls.selectTone(f.token(),id(8))
            assertFalse(f.controls.edit(f.token(),f.form().copy(gamma="2")))
            assertFalse(f.controls.apply(f.token())); assertFalse(f.controls.remove(f.token())); assertEquals(0,f.prepares)
            f.tones = f.tones + f.tones
            f.controls.discardFormAndReload(f.token()); f.idle()
            assertNull(f.controls.state.value.snapshot); assertNotNull(f.controls.state.value.issue)
        } finally { f.close() }
    }

    @Test fun applyRejectsSynchronousIdFactoryRebindWithoutInstallingOldDraftIdsOrState() = runBlocking {
        for (replacementProject in listOf(false,true)) for (trigger in listOf(1,2)) for (fail in listOf(false,true)) {
            val f = Fixture()
            try {
                f.open(); f.controls.edit(f.token(),f.form().copy(brightness="0.25"))
                val old = f.token()
                val replacement = if (replacementProject) object : UnsupportedProject() {
                    override suspend fun info(): ProjectInfo = f.info
                } else f.project
                f.idHook = { number -> if (number == trigger) {
                    f.idHook = null
                    f.controls.bind(replacement,f.document(),if (replacementProject) 0 else 1,0uL)
                    if (fail) throw IllegalStateException("Old draft ID allocation failed after rebind")
                } }
                assertFalse(f.controls.apply(old)); f.idle()
                assertEquals("0.0",f.form().brightness); assertNull(f.controls.state.value.issue)
                assertFalse(f.controls.state.value.dirty); assertFalse(f.controls.state.value.busy)
                assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares); assertEquals(0,f.commits)
                val next = 100+f.ids
                assertTrue(f.controls.preview(f.token(),0u,0u,false)); f.previewIdle()
                val draft = f.previewCommands.single() as ToneControlCommand.Set
                assertEquals(id(next),draft.layerId); assertEquals(id(next+1),draft.objectId)
                assertEquals(next-100+2,f.ids)
                assertTrue(f.controls.apply(f.token())); f.idle()
                assertEquals(draft,f.commands.single())
            } finally { f.close() }
        }
    }

    @Test fun previewRejectsSynchronousIdFactoryRebindWithoutPublishingOldFormOrStuckLoading() = runBlocking {
        for (replacementProject in listOf(false,true)) for (trigger in listOf(1,2)) for (fail in listOf(false,true)) {
            val f = Fixture()
            try {
                f.open(); f.controls.edit(f.token(),f.form().copy(contrast="0.25"))
                val old = f.token()
                val replacement = if (replacementProject) object : UnsupportedProject() {
                    override suspend fun info(): ProjectInfo = f.info
                } else f.project
                f.idHook = { number -> if (number == trigger) {
                    f.idHook = null
                    f.controls.bind(replacement,f.document(),if (replacementProject) 0 else 1,0uL)
                    if (fail) throw IllegalStateException("Old preview ID allocation failed after rebind")
                } }
                assertFalse(f.controls.preview(old,0u,0u,false)); f.idle()
                assertEquals("0.0",f.form().contrast); assertNull(f.controls.state.value.issue)
                assertFalse(f.controls.state.value.previewLoading); assertNull(f.controls.state.value.preview)
                assertEquals(0,f.previews); assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares)
                val next = 100+f.ids
                assertTrue(f.controls.preview(f.token(),0u,0u,false)); f.previewIdle()
                val draft = f.previewCommands.single() as ToneControlCommand.Set
                assertEquals(id(next),draft.layerId); assertEquals(id(next+1),draft.objectId)
                assertEquals(next-100+2,f.ids)
            } finally { f.close() }
        }
    }

    @Test fun reentrantPreviewKeepsItsOwnIdsWhenAnOlderAllocationReturns() = runBlocking {
        for (apply in listOf(false,true)) {
            val f = Fixture()
            try {
                f.open()
                f.idHook = {
                    f.idHook = null
                    assertTrue(f.controls.preview(f.token(),1u,2u,false))
                }
                val old = f.token()
                assertFalse(if (apply) f.controls.apply(old) else f.controls.preview(old,0u,0u,false))
                f.previewIdle()
                assertEquals(0,f.prepares); assertEquals(4,f.ids)
                val owned = f.previewCommands.single() as ToneControlCommand.Set
                assertEquals(id(101),owned.layerId); assertEquals(id(102),owned.objectId)
                assertTrue(f.controls.preview(f.token(),3u,4u,false)); f.previewIdle()
                assertEquals(owned,f.previewCommands.last()); assertEquals(4,f.ids)
                assertTrue(f.controls.apply(f.token())); f.idle()
                assertEquals(owned,f.commands.single()); assertEquals(4,f.ids)
            } finally { f.close() }
        }
    }

    private class Fixture {
        val scope = CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val mutations = Mutex()
        var info = ProjectInfo(id(1),"Project",id(5),1uL,false,false,0uL,"a".repeat(64),listOf(id(2)))
        var tones: List<ToneControlEntry> = listOf(ToneControlEntry(id(7),id(8),true,false,1.0,ImageToneSettings()))
        var capabilities = 0; var reads = 0; var previews = 0; var previewFinished = 0
        var ids = 0; var metadataCalls = 0; var prepares = 0; var commits = 0; var closedPlans = 0; var refreshCalls = 0
        var readGate: (suspend () -> Unit)? = null
        var previewGate: (suspend (Int) -> Unit)? = null
        var metadataGate: (suspend () -> Unit)? = null
        var commitGate: (suspend () -> Unit)? = null
        var idHook: ((Int) -> Unit)? = null
        var readFailure: Exception? = null
        var corruptPreview: ((ToneControlPreview) -> ToneControlPreview)? = null
        var lastRawPreview: ToneControlPreview? = null
        var failFirstCommit = false; var failRefresh = false
        val commands = mutableListOf<ToneControlCommand>()
        val previewCommands = mutableListOf<ToneControlCommand?>()
        val previewConsents = mutableListOf<Boolean>()
        val project = object : UnsupportedProject() { override suspend fun info(): ProjectInfo = this@Fixture.info }
        val controls = ToneControlsInteraction(scope,mutations,{
            val value = id(100+ids++); idHook?.invoke(ids); value
        },{
            metadataCalls++; metadataGate?.invoke(); WorkflowMetadata(id(60+metadataCalls),info.deviceId,info.nextLamport,1700000000000)
        },{ true },{
            refreshCalls++; if (failRefresh) throw IllegalStateException("refresh failed")
        },{
            capabilities++
            object : WorkbenchToneControls {
                override suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong): ToneControlSnapshot {
                    reads++; readFailure?.let { throw it }
                    val value = this@Fixture.snapshot(); readGate?.invoke(); return value
                }
                override suspend fun preview(binding: WorkflowBinding, command: ToneControlCommand?, region: SelectionRegion,
                    assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong): ToneControlPreview {
                    val number = ++previews; previewCommands += command; previewConsents += assumeUntaggedSrgb
                    try {
                        val value = ToneControlPreview(binding,"c".repeat(64),region.x,region.y,region.width,region.height,
                            ByteArray((region.width*region.height*4u).toInt()) { number.toByte() })
                        lastRawPreview = value
                        previewGate?.invoke(number)
                        return corruptPreview?.invoke(value) ?: value
                    } finally { previewFinished++ }
                }
                override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ToneControlCommand,
                    memoryBudgetBytes: ULong): WorkbenchWorkflowPlan {
                    prepares++; commands += command
                    return object : WorkbenchWorkflowPlan {
                        var closed = false
                        override fun describe(): WorkflowPlanInfo = WorkflowPlanInfo(metadata.transactionId,binding,1u,metadata.firstLamport+1uL)
                        override suspend fun commit(): WorkflowReceipt {
                            commits++; commitGate?.invoke()
                            if (failFirstCommit && commits == 1) throw WorkflowFailure(WorkflowFailureKind.Storage)
                            return WorkflowReceipt(metadata.transactionId,info,duplicate=commits > 1)
                        }
                        override fun close() { if (!closed) { closed = true; closedPlans++ } }
                    }
                }
            }
        })
        fun document(): DocumentSnapshot = DocumentSnapshot(id(2),"Original",1024u,768u,8u,emptyList(),RenderList(info,emptyList()))
        fun snapshot(): ToneControlSnapshot = ToneControlSnapshot(WorkflowBinding(info.projectId,id(2),info.hostSeq,info.stateHash),
            "c".repeat(64),1024u,768u,8u,tones)
        fun token(): ToneControlsToken = controls.capture()!!
        fun form(): ToneControlForm = controls.state.value.form!!
        fun publish(sequence: ULong = 0uL) { controls.bind(project,document(),0,sequence) }
        suspend fun open() { publish(); controls.active(true); idle() }
        suspend fun idle() { withTimeout(3000) { while (controls.state.value.busy || controls.state.value.loading) yield() } }
        suspend fun previewIdle() { withTimeout(3000) { while (controls.state.value.previewLoading) yield() } }
        suspend fun close() { controls.close(); scope.cancel() }
    }

    private abstract class UnsupportedProject : WorkbenchProject {
        override val changes = emptyFlow<ProjectChange>()
        override suspend fun info(): ProjectInfo = error("unused")
        override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
        override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
        override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
        override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
        override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
        override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
        override suspend fun export(options: ExportOptions): ExportResult = error("unused")
        override suspend fun close(): Unit = Unit
    }
    companion object { private fun id(number: Int): String = "00000000-0000-4000-8000-"+number.toString().padStart(12,'0') }
}
