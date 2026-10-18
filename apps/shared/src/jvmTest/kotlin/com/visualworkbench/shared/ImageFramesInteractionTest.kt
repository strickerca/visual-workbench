package com.visualworkbench.shared

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

class ImageFramesInteractionTest {
    @Test fun readsRemainLazyUntilActivationAndWholeInvalidDraftsRemainPrivate() = runBlocking {
        val f = Fixture()
        try {
            f.publish(); repeat(3) { yield() }; assertEquals(0,f.capabilities)
            f.controls.active(true); f.idle(); assertEquals(1,f.reads); assertTrue(f.controls.state.value.ready)
            assertFalse(f.controls.state.value.previewAssumeSrgb)
            val defaults = f.form()
            for (invalid in listOf(defaults.copy(cropWidth="0"),defaults.copy(cropX="1024"),
                defaults.copy(outputWidth="32769"),defaults.copy(outputWidth="10000",outputHeight="5001"),
                defaults.copy(outputWidth="-"),defaults.copy(cropHeight="4294967296"))) {
                assertTrue(f.controls.edit(f.token(),invalid))
                assertFalse(f.controls.apply(f.token())); assertFalse(f.controls.preview(f.token(),0u,0u,false))
            }
            assertFalse(f.controls.edit(f.token(),defaults.copy(cropX="1".repeat(33))))
            assertEquals("4294967296",f.form().cropHeight)
            assertEquals(0,f.ids); assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares); assertEquals(0,f.previews)
            assertFalse(f.controls.state.value.mayLeave); assertFalse(f.controls.active(false)); assertFalse(f.controls.remove(f.token()))
            assertTrue(f.controls.discardFormAndReload(f.token())); f.idle(); assertTrue(f.controls.active(false))
        } finally { f.close() }
    }

    @Test fun exactPrivateFramePreviewHasItsOwnOutputGridAndStableIdsThenOneExplicitApply() = runBlocking {
        val f = Fixture()
        try {
            f.open()
            f.controls.edit(f.token(),f.form().copy(cropX="20",cropY="30",cropWidth="400",cropHeight="300",
                clockwiseDegrees=90u,outputWidth="1500",outputHeight="10"))
            assertTrue(f.controls.preview(f.token(),1100u,0u,false,marked=false)); f.previewIdle()
            val first = f.previewCommands.single() as ImageFrameCommand.Set
            assertEquals(ImageFrameSettings(20u,30u,400u,300u,90u,1500u,10u),first.settings)
            assertEquals(400u,f.controls.state.value.preview!!.width)
            assertFalse(f.controls.preview(f.token(),1500u,0u,false)); assertEquals(2,f.ids)
            assertTrue(f.controls.preview(f.token(),1000u,1u,true)); f.previewIdle()
            assertEquals(first,f.previewCommands.last()); assertEquals(listOf(false,true),f.previewMarks)
            assertEquals(0,f.prepares); assertEquals(0,f.commits); assertTrue(f.controls.state.value.dirty)
            f.mutations.lock(); assertTrue(f.controls.apply(f.token())); assertFalse(f.controls.apply(f.token()))
            assertEquals(0,f.prepares); f.mutations.unlock(); f.idle()
            assertEquals(first,f.commands.single()); assertEquals(1,f.metadataCalls); assertEquals(1,f.prepares)
            assertEquals(1,f.commits); assertEquals(1,f.closedPlans); assertNotNull(f.controls.state.value.lastReceipt)
            assertEquals(2,f.ids)
        } finally { if (f.mutations.isLocked) f.mutations.unlock(); f.close() }
    }

    @Test fun savedPreviewAndSourceEditingViewDoNotValidateOrCommitPrivateTyping() = runBlocking {
        val f = Fixture()
        try {
            f.frame = savedFrame(); f.open()
            f.controls.edit(f.token(),f.form().copy(outputWidth=""))
            assertTrue(f.controls.preview(f.token(),0u,0u,false,draft=false)); f.previewIdle()
            assertNull(f.previewCommands.single()); assertEquals(80u,f.controls.state.value.preview!!.outputWidth)
            assertFalse(f.controls.preview(f.token(),900u,700u,false,draft=false))
            assertTrue(f.controls.editingView(f.token(),900u,700u,true)); f.previewIdle()
            assertNull(f.controls.state.value.preview)
            assertEquals(900u,f.controls.state.value.editingPreview!!.x)
            assertEquals(124u,f.controls.state.value.editingPreview!!.width)
            assertEquals(68u,f.controls.state.value.editingPreview!!.height)
            assertTrue(f.controls.state.value.dirty); assertEquals(0,f.ids); assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares)
        } finally { f.close() }
    }

    @Test fun disableRetainsSettingsAndRemoveTargetsOnlyTheSavedFrame() = runBlocking {
        val f = Fixture()
        try {
            f.frame = savedFrame(); f.open()
            assertTrue(f.controls.edit(f.token(),f.form().copy(enabled=false)))
            assertFalse(f.controls.remove(f.token()))
            assertTrue(f.controls.preview(f.token(),900u,700u,false)); f.previewIdle()
            assertNull(f.controls.state.value.preview!!.frame)
            assertEquals(1024u,f.controls.state.value.preview!!.outputWidth)
            assertTrue(f.controls.apply(f.token())); f.idle()
            val command = f.commands.single() as ImageFrameCommand.Set
            assertEquals(f.frame!!.settings,command.settings); assertFalse(command.enabled); assertEquals(0,f.ids)
            assertTrue(f.controls.remove(f.token())); f.idle()
            assertEquals(ImageFrameCommand.Remove(id(7),id(8)),f.commands.last())
            assertEquals(2,f.prepares); assertEquals(2,f.commits)
        } finally { f.close() }
    }

    @Test fun locksRefuseEditingApplyingAndRemovingButAllowSavedReads() = runBlocking {
        val f = Fixture()
        try {
            f.frame = savedFrame().copy(locked=true); f.open()
            assertFalse(f.controls.edit(f.token(),f.form().copy(cropWidth="10")))
            assertFalse(f.controls.apply(f.token())); assertFalse(f.controls.remove(f.token()))
            assertFalse(f.controls.preview(f.token(),0u,0u,false))
            assertTrue(f.controls.preview(f.token(),0u,0u,false,draft=false)); f.previewIdle()
            assertTrue(f.controls.editingView(f.token(),900u,700u,false)); f.previewIdle()
            assertEquals(0,f.ids); assertEquals(0,f.prepares)
        } finally { f.close() }
    }

    @Test fun dirtyFormRemainsStaleAcrossObservedSameHashAndPageEpochCallbacksAreRejected() = runBlocking {
        val f = Fixture()
        try {
            f.open(); val old = f.token()
            f.controls.edit(old,f.form().copy(cropWidth="400")); f.controls.observe(1uL)
            assertNull(f.controls.capture()); f.publish(1uL); f.idle()
            assertTrue(f.controls.state.value.stale); assertEquals("400",f.form().cropWidth)
            assertFalse(f.controls.apply(old)); assertFalse(f.controls.apply(f.token()))
            assertTrue(f.controls.discardFormAndReload(f.token())); f.idle()
            val beforePage = f.token()
            f.controls.bind(f.project,f.document().copy(page=1u),0,1uL); f.idle()
            assertFalse(f.controls.apply(beforePage)); assertFalse(f.controls.edit(beforePage,f.form()))
            assertFalse(f.controls.preview(beforePage,0u,0u,false)); assertFalse(f.controls.editingView(beforePage,0u,0u,false))
            val beforeEpoch = f.token()
            f.controls.bind(f.project,f.document().copy(page=1u),1,1uL); f.idle()
            assertFalse(f.controls.apply(beforeEpoch)); assertEquals(0,f.prepares)
        } finally { f.close() }
    }

    @Test fun failedCommitRetriesTheExactPlanAndReceiptSurvivesFallibleRefresh() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.failFirstCommit = true; f.failRefresh = true
            f.controls.edit(f.token(),f.form().copy(outputWidth="401",outputHeight="301"))
            assertTrue(f.controls.apply(f.token())); f.idle()
            val retained = f.controls.state.value.retry!!
            assertEquals(1,f.prepares); assertEquals(1,f.metadataCalls); assertEquals(0,f.closedPlans)
            assertFalse(f.controls.apply(f.token())); assertFalse(f.controls.active(false))
            assertTrue(f.controls.retry(f.token())); f.idle()
            assertEquals(1,f.prepares); assertEquals(1,f.metadataCalls); assertEquals(2,f.commits); assertEquals(1,f.closedPlans)
            assertEquals(retained.transactionId,f.controls.state.value.lastReceipt!!.transactionId)
            assertTrue(f.controls.state.value.lastReceipt!!.duplicate); assertNull(f.controls.state.value.retry)
            assertFalse(f.controls.state.value.dirty); assertNotNull(f.controls.state.value.issue)
        } finally { f.close() }
    }

    @Test fun oneReaderConflatesOutputAndSourceReadsAndNeverPublishesSupersededPixels() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.open(); f.previewGate = { if (it == 1) { entered.complete(Unit); release.await() } }
            f.controls.preview(f.token(),0u,0u,false); entered.await()
            for (x in 1u..20u) assertTrue(f.controls.preview(f.token(),x,3u,true))
            assertTrue(f.controls.editingView(f.token(),900u,700u,false))
            assertEquals(1,f.previews); assertEquals(0,f.editingReads)
            release.complete(Unit); f.previewIdle()
            assertEquals(1,f.previews); assertEquals(1,f.editingReads)
            assertNull(f.controls.state.value.preview); assertEquals(900u,f.controls.state.value.editingPreview!!.x)
            f.controls.invalidatePreview(f.token()); assertNull(f.controls.state.value.editingPreview)
            assertEquals(0,f.prepares)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun returnedPixelsAreOwnedAndWrongSourceFrameGridRevisionAndRegionAreRefused() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.controls.preview(f.token(),0u,0u,false); f.previewIdle()
            val accepted = f.controls.state.value.preview!!; f.rawPreview!!.rgba.fill(9)
            assertEquals(1.toByte(),accepted.rgba[0]); assertTrue(accepted.toString().contains("rgbaBytes="))
            assertFalse(accepted.toString().contains("rgba=["))
            val corruptions: List<(ImageFramePreview) -> ImageFramePreview> = listOf(
                { it.copy(sourceAssetId="d".repeat(64)) },{ it.copy(frame=null) },{ it.copy(outputWidth=1u) },
                { it.copy(binding=it.binding.copy(hostSeq=2uL)) },{ it.copy(x=1u) },{ it.copy(rgba=ByteArray(4)) })
            for (corrupt in corruptions) {
                f.corruptPreview = corrupt; assertTrue(f.controls.preview(f.token(),0u,0u,false)); f.previewIdle()
                assertNull(f.controls.state.value.preview); assertNotNull(f.controls.state.value.issue)
            }
            assertFalse(f.controls.preview(f.token(),SelectionRegion(UInt.MAX_VALUE,0u,2u,1u),false))
            assertFalse(f.controls.editingView(f.token(),SelectionRegion(0u,0u,513u,1u),false))
            f.corruptEditing = { it.copy(sourceAssetId="e".repeat(64)) }
            assertTrue(f.controls.editingView(f.token(),0u,0u,false)); f.previewIdle()
            assertNull(f.controls.state.value.editingPreview); assertNotNull(f.controls.state.value.issue)
        } finally { f.close() }
    }

    @Test fun idFactoryRebindCannotInstallOldIdsOrOldStateForEitherApplyOrPreview() = runBlocking {
        for (apply in listOf(false,true)) for (trigger in listOf(1,2)) for (fail in listOf(false,true)) {
            val f = Fixture()
            try {
                f.open(); f.controls.edit(f.token(),f.form().copy(outputWidth="400")); val old = f.token()
                f.idHook = { number -> if (number == trigger) {
                    f.idHook = null; f.controls.bind(f.project,f.document(),1,0uL)
                    if (fail) throw IllegalStateException("Old allocation failed after rebind")
                } }
                assertFalse(if (apply) f.controls.apply(old) else f.controls.preview(old,0u,0u,false)); f.idle()
                assertEquals("1024",f.form().outputWidth); assertNull(f.controls.state.value.issue)
                assertFalse(f.controls.state.value.busy); assertFalse(f.controls.state.value.previewLoading)
                assertFalse(f.controls.state.value.dirty); assertEquals(0,f.prepares); assertEquals(0,f.previews)
                val next = 100+f.ids
                assertTrue(f.controls.preview(f.token(),0u,0u,false)); f.previewIdle()
                val command = f.previewCommands.single() as ImageFrameCommand.Set
                assertEquals(id(next),command.layerId); assertEquals(id(next+1),command.objectId)
                assertTrue(f.controls.apply(f.token())); f.idle(); assertEquals(command,f.commands.single())
            } finally { f.close() }
        }
    }

    @Test fun nestedNewPreviewOwnsItsIdsWhenTheOlderIdAllocationReturns() = runBlocking {
        for (apply in listOf(false,true)) {
            val f = Fixture()
            try {
                f.open(); f.idHook = {
                    f.idHook = null; assertTrue(f.controls.preview(f.token(),1u,2u,false))
                }
                val old = f.token()
                assertFalse(if (apply) f.controls.apply(old) else f.controls.preview(old,0u,0u,false)); f.previewIdle()
                val command = f.previewCommands.single() as ImageFrameCommand.Set
                assertEquals(id(101),command.layerId); assertEquals(id(102),command.objectId); assertEquals(4,f.ids)
                assertTrue(f.controls.apply(f.token())); f.idle(); assertEquals(command,f.commands.single()); assertEquals(4,f.ids)
            } finally { f.close() }
        }
    }

    @Test fun detachJoinsActualReadAndCommitSettlementBeforeReleasingExactPlan() = runBlocking {
        val f = Fixture(); val reading = CompletableDeferred<Unit>(); val releaseRead = CompletableDeferred<Unit>()
        val writing = CompletableDeferred<Unit>(); val releaseWrite = CompletableDeferred<Unit>()
        var detached: Deferred<Unit>? = null
        try {
            f.open()
            f.previewGate = { withContext(NonCancellable) { reading.complete(Unit); releaseRead.await() }; currentCoroutineContext().ensureActive() }
            f.commitGate = { withContext(NonCancellable) { writing.complete(Unit); releaseWrite.await() }; currentCoroutineContext().ensureActive() }
            f.controls.preview(f.token(),0u,0u,false); reading.await()
            f.controls.apply(f.token()); writing.await(); f.controls.cancel(f.token())
            assertTrue(f.controls.state.value.busy); assertFalse(f.controls.active(false))
            detached = async(start=CoroutineStart.UNDISPATCHED) { f.controls.detach() }
            assertFalse(detached.isCompleted); assertEquals(0,f.closedPlans)
            releaseRead.complete(Unit); repeat(3) { yield() }; assertFalse(detached.isCompleted)
            releaseWrite.complete(Unit); withTimeout(3000) { detached.await() }
            assertEquals(1,f.previewFinished); assertEquals(1,f.closedPlans)
            assertNull(f.controls.state.value.retry); assertNull(f.controls.state.value.preview)
        } finally { releaseRead.complete(Unit); releaseWrite.complete(Unit); withContext(NonCancellable) { detached?.join(); f.close() } }
    }

    @Test fun lateAcceptedOldAttachmentNeverRefreshesItsReplacement() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        try {
            f.open(); f.commitGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
            f.controls.apply(f.token()); entered.await(); f.controls.bind(f.project,f.document(),1,0uL)
            release.complete(Unit); f.idle()
            assertNull(f.controls.state.value.lastReceipt); assertEquals(0,f.refreshCalls)
            assertEquals(1,f.commits); assertEquals(1,f.closedPlans)
        } finally { release.complete(Unit); f.close() }
    }

    @Test fun cancelledParentRejectsCallbacksAndEveryCloseWaitsForTheSourceReader() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var first: Deferred<Unit>? = null; var second: Deferred<Unit>? = null
        try {
            f.open(); f.editingGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
            val old = f.token(); f.controls.editingView(old,0u,0u,false); entered.await(); f.scope.cancel()
            assertNull(f.controls.capture()); assertFalse(f.controls.active(true)); assertFalse(f.controls.apply(old))
            assertFalse(f.controls.preview(old,0u,0u,false)); assertFalse(f.controls.editingView(old,0u,0u,false))
            first = async(start=CoroutineStart.UNDISPATCHED) { f.controls.close() }
            second = async(start=CoroutineStart.UNDISPATCHED) { f.controls.close() }
            assertFalse(first.isCompleted); assertFalse(second.isCompleted)
            release.complete(Unit); withTimeout(3000) { first.await(); second.await() }
            assertEquals(1,f.editingFinished); assertNull(f.controls.state.value.editingPreview); assertEquals(0,f.prepares)
        } finally { release.complete(Unit); withContext(NonCancellable) { first?.join(); second?.join(); f.close() } }
    }

    @Test fun operationCancellationDoesNotKillTheLongLivedReader() = runBlocking {
        val f = Fixture()
        try {
            f.readFailure = CancellationException("one operation cancelled"); f.open()
            assertNull(f.controls.state.value.snapshot); assertNotNull(f.controls.state.value.issue)
            f.readFailure = null; f.controls.discardFormAndReload(f.token()); f.idle(); assertTrue(f.controls.state.value.ready)
            f.previewGate = { throw CancellationException("one preview cancelled") }
            f.controls.preview(f.token(),0u,0u,false); f.previewIdle(); assertNull(f.controls.state.value.preview)
            f.previewGate = null; f.controls.preview(f.token(),0u,0u,false); f.previewIdle(); assertNotNull(f.controls.state.value.preview)
            assertEquals(2,f.reads); assertEquals(2,f.previews)
        } finally { f.close() }
    }

    @Test fun departureImmediatelyRefusesRetainedCallbacksAndLateBindUntilJoinedDetachThenReopens() = runBlocking {
        val f = Fixture(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        var detached: Deferred<Unit>? = null
        try {
            f.frame = savedFrame(); f.open(); val old = f.token(); val form = f.form()
            f.editingGate = { withContext(NonCancellable) { entered.complete(Unit); release.await() } }
            assertTrue(f.controls.editingView(old,0u,0u,false)); entered.await()
            f.controls.beginDeparture()
            assertNull(f.controls.capture()); assertTrue(f.controls.state.value.departing)
            assertFalse(f.controls.state.value.ready); assertFalse(f.controls.state.value.previewLoading)
            val fenced = f.controls.state.value
            assertFalse(f.controls.edit(old,form.copy(outputWidth="40")))
            assertFalse(f.controls.apply(old)); assertFalse(f.controls.remove(old)); assertFalse(f.controls.retry(old))
            assertFalse(f.controls.discardRetry(old)); assertFalse(f.controls.discardFormAndReload(old))
            assertFalse(f.controls.preview(old,0u,0u,false))
            assertFalse(f.controls.preview(old,SelectionRegion(0u,0u,1u,1u),false))
            assertFalse(f.controls.editingView(old,0u,0u,false))
            assertFalse(f.controls.editingView(old,SelectionRegion(0u,0u,1u,1u),false))
            assertFalse(f.controls.active(false)); assertFalse(f.controls.active(true))
            f.controls.invalidatePreview(old); f.controls.cancel(old); f.controls.observe(1uL)
            f.controls.bind(f.project,f.document(),1,1uL)
            assertSame(fenced,f.controls.state.value)
            assertEquals(0,f.ids); assertEquals(0,f.metadataCalls); assertEquals(0,f.prepares)
            assertEquals(1,f.reads); assertEquals(0,f.previews); assertEquals(1,f.editingReads)
            detached = async(start=CoroutineStart.UNDISPATCHED) { f.controls.detach() }
            assertFalse(detached.isCompleted)
            f.controls.bind(f.project,f.document(),1,1uL)
            assertNull(f.controls.capture()); assertFalse(f.controls.active(true)); assertEquals(1,f.reads)
            release.complete(Unit); withTimeout(3000) { detached.await() }
            assertFalse(f.controls.state.value.departing); assertNull(f.controls.state.value.editingPreview)
            f.editingGate = null; f.controls.bind(f.project,f.document(),1,1uL)
            assertTrue(f.controls.active(true)); f.idle(); assertTrue(f.controls.state.value.ready)
            assertNotEquals(old,f.token()); assertEquals(2,f.reads)
            assertTrue(f.controls.preview(f.token(),0u,0u,false)); f.previewIdle()
            assertNotNull(f.controls.state.value.preview); assertEquals(1,f.previews)
            assertTrue(f.controls.editingView(f.token(),900u,700u,false)); f.previewIdle()
            assertNotNull(f.controls.state.value.editingPreview); assertEquals(2,f.editingReads)
        } finally { release.complete(Unit); withContext(NonCancellable) { detached?.join(); f.close() } }
    }

    @Test fun departureRefusesRetryAndDiscardWhileTheExactPlanStillAwaitsJoinedRetirement() = runBlocking {
        val f = Fixture()
        try {
            f.open(); f.failFirstCommit = true
            assertTrue(f.controls.apply(f.token())); f.idle()
            val old = f.token(); val retained = f.controls.state.value.retry
            assertNotNull(retained); f.controls.beginDeparture()
            assertFalse(f.controls.retry(old)); assertFalse(f.controls.discardRetry(old))
            assertFalse(f.controls.apply(old)); assertFalse(f.controls.discardFormAndReload(old))
            assertEquals(retained,f.controls.state.value.retry); assertEquals(0,f.closedPlans)
            f.controls.detach(); assertEquals(1,f.closedPlans); assertNull(f.controls.state.value.retry)
            f.publish(); assertTrue(f.controls.active(true)); f.idle()
            assertTrue(f.controls.state.value.ready); assertTrue(f.controls.apply(f.token())); f.idle()
            assertEquals(2,f.prepares); assertEquals(2,f.commits); assertEquals(2,f.closedPlans)
        } finally { f.close() }
    }

    private class Fixture {
        val scope = CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val mutations = Mutex()
        var info = ProjectInfo(id(1),"Project",id(5),1uL,false,false,0uL,"a".repeat(64),listOf(id(2)))
        var frame: ImageFrameEntry? = null
        var capabilities = 0; var reads = 0; var previews = 0; var previewFinished = 0
        var editingReads = 0; var editingFinished = 0
        var ids = 0; var metadataCalls = 0; var prepares = 0; var commits = 0; var closedPlans = 0; var refreshCalls = 0
        var previewGate: (suspend (Int) -> Unit)? = null
        var editingGate: (suspend () -> Unit)? = null
        var commitGate: (suspend () -> Unit)? = null
        var idHook: ((Int) -> Unit)? = null
        var readFailure: Exception? = null
        var corruptPreview: ((ImageFramePreview) -> ImageFramePreview)? = null
        var corruptEditing: ((ImageEditingPreview) -> ImageEditingPreview)? = null
        var rawPreview: ImageFramePreview? = null
        var failFirstCommit = false; var failRefresh = false
        val commands = mutableListOf<ImageFrameCommand>()
        val previewCommands = mutableListOf<ImageFrameCommand?>()
        val previewMarks = mutableListOf<Boolean>()
        val project = object : UnsupportedProject() { override suspend fun info(): ProjectInfo = this@Fixture.info }
        val controls = ImageFramesInteraction(scope,mutations,{
            val value = id(100+ids++); idHook?.invoke(ids); value
        },{
            metadataCalls++; WorkflowMetadata(id(60+metadataCalls),info.deviceId,info.nextLamport,1700000000000)
        },{ true },{
            refreshCalls++; if (failRefresh) throw IllegalStateException("refresh failed")
        },{
            capabilities++
            object : WorkbenchImageFrames {
                override suspend fun snapshot(documentId: String, memoryBudgetBytes: ULong): ImageFramesSnapshot {
                    reads++; readFailure?.let { throw it }; return this@Fixture.snapshot()
                }
                override suspend fun preview(binding: WorkflowBinding, command: ImageFrameCommand?, region: SelectionRegion,
                    marked: Boolean, assumeUntaggedSrgb: Boolean, memoryBudgetBytes: ULong): ImageFramePreview {
                    val number = ++previews; previewCommands += command; previewMarks += marked
                    try {
                        val effective = this@Fixture.snapshot().effectiveFrame(command)
                        val value = ImageFramePreview(binding,"c".repeat(64),effective,effective?.outputWidth ?: 1024u,
                            effective?.outputHeight ?: 768u,region.x,region.y,region.width,region.height,
                            ByteArray((region.width*region.height*4u).toInt()) { number.toByte() })
                        rawPreview = value; previewGate?.invoke(number); return corruptPreview?.invoke(value) ?: value
                    } finally { previewFinished++ }
                }
                override suspend fun editingView(binding: WorkflowBinding, region: SelectionRegion, assumeUntaggedSrgb: Boolean,
                    memoryBudgetBytes: ULong): ImageEditingPreview {
                    editingReads++
                    try {
                        val value = ImageEditingPreview(binding,"c".repeat(64),region.x,region.y,region.width,region.height,
                            ByteArray((region.width*region.height*4u).toInt()) { 1 })
                        editingGate?.invoke(); return corruptEditing?.invoke(value) ?: value
                    } finally { editingFinished++ }
                }
                override suspend fun editingBackground(binding: WorkflowBinding, assumeUntaggedSrgb: Boolean,
                    memoryBudgetBytes: ULong): ImageEditingBackground = error("unused by the private-frame interaction")
                override suspend fun prepare(binding: WorkflowBinding, metadata: WorkflowMetadata, command: ImageFrameCommand,
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
        fun snapshot(): ImageFramesSnapshot = ImageFramesSnapshot(WorkflowBinding(info.projectId,id(2),info.hostSeq,info.stateHash),
            "c".repeat(64),1024u,768u,8u,frame)
        fun token(): ImageFramesToken = controls.capture()!!
        fun form(): ImageFramesForm = controls.state.value.form!!
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
    companion object {
        private fun savedFrame(): ImageFrameEntry = ImageFrameEntry(id(7),id(8),true,false,
            ImageFrameSettings(100u,100u,200u,100u,90u,80u,50u))
        private fun id(number: Int): String = "00000000-0000-4000-8000-"+number.toString().padStart(12,'0') }
}
