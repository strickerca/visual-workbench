package com.visualworkbench.shared

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantViewportSessionTest {
    private val binding = WorkflowBinding("project", "document", 1uL, "c".repeat(64))
    private fun view(x: Double) = GiantViewport(x,0.0,x+256.0,256.0,1.0,0.0,0.0,1.0)
    private class Source : GiantImage {
        val reads = mutableListOf<GiantTileAddress>()
        var viewportCalls = 0
        var identity = "a".repeat(64)
        var onRead: suspend () -> Unit = {}
        var closed = false
        var many = false
        var level: UByte = 0u
        var documentOffset: UInt = 0u
        override suspend fun viewport(view: GiantViewport): GiantViewportPlan {
            viewportCalls++
            val keys = if(many)(0u..2u).map{GiantTileAddress(level,it,0u)} else listOf(GiantTileAddress(level,view.left.toUInt(),0u))
            return GiantViewportPlan(identity,"b".repeat(64),level,keys,emptyList())
        }
        override suspend fun tile(address: GiantTileAddress, foreign: SessionResourceCharge): GiantTilePixels {
            reads += address; onRead()
            return pixels(address)
        }
        fun pixels(address: GiantTileAddress) = GiantTilePixels(identity,"b".repeat(64),address,
            address.x*256u+documentOffset,0u,256u,256u,256u,256u,
            ByteArray(256*256*4){if(it%4==3)255.toByte() else 0})
        override suspend fun close(){closed=true}
    }
    private class EditingSource(val source: Source = Source()) : GiantImage by source, GiantEditingImage {
        val edits = mutableListOf<Pair<GiantTileAddress, Boolean>>()
        var onEdit: suspend () -> Unit = {}
        override suspend fun editingTile(address: GiantTileAddress, foreign: SessionResourceCharge,
            assumeUntaggedSrgb: Boolean): GiantTilePixels {
            edits += address to assumeUntaggedSrgb
            onEdit()
            return source.pixels(address)
        }
    }
    private class Renderer : GiantRenderBackend<Int> {
        val events = mutableListOf<String>(); var next=0; var frame:GiantVisibleFrame<Int>?=null
        var onRetire:suspend()->Unit={}
        var onPresent:(GiantVisibleFrame<Int>?)->Unit={}
        override suspend fun create(pixels:GiantTilePixels):Int=(++next).also{events+="create:$it"}
        override fun present(frame:GiantVisibleFrame<Int>?){onPresent(frame);this.frame=frame;events+="present:${frame?.generation}"}
        override suspend fun retire(){events+="retire:start";onRetire();events+="retire:end"}
        override fun release(image:Int){events+="release:$image"}
    }
    private class Resources(private val limit: ULong = 512uL * 1024uL * 1024uL) : SessionResources {
        var used = 0uL; var closes = 0; var failClose = false
        override suspend fun reserveForeign(bytes: ULong): SessionResourceCharge {
            if (bytes > limit - used) throw GiantImageFailure(GiantImageFailureKind.Memory)
            used += bytes
            return object : SessionResourceCharge {
                private var closed = false
                override fun close() {
                    if (closed) return
                    if (failClose) throw IllegalStateException("reservation is retained")
                    closed = true; used -= bytes; closes++
                }
            }
        }
    }
    private fun session(parent: CoroutineScope, image: GiantImage, backend: GiantRenderBackend<Int>,
        bound: WorkflowBinding, epoch: Long, current: (WorkflowBinding, Long) -> Boolean,
        max: Long = 192L * 1024L * 1024L, resources: Resources = Resources(),
        content: GiantTileContent = GiantTileContent.Source,
        assumeUntaggedSrgb: Boolean = false): GiantViewportSession<Int> =
        GiantViewportSession(parent, image, backend, bound, epoch, current, resources, max, content, assumeUntaggedSrgb)
    private suspend fun until(test:()->Boolean){withTimeout(3000){while(!test())yield()}}
    @Test fun evictionCreditsOnlyAfterActualRendererRetirement()=runBlocking {
        val source=Source();val backend=Renderer();val session=session(this,source,backend,binding,1L,{_,_->true},GiantViewportSession.TILE_CHARGE)
        val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        try{
            session.request(view(0.0));until{session.state.value.ready}
            backend.onRetire={entered.complete(Unit);leave.await()}
            session.request(view(1.0));withTimeout(3000){entered.await()}
            assertEquals(GiantViewportSession.TILE_CHARGE,session.state.value.retainedBytes)
            assertFalse(backend.events.any{it.startsWith("release:")})
            leave.complete(Unit);until{session.state.value.ready}
            assertTrue(backend.events.indexOf("retire:end")<backend.events.indexOf("release:1"))
            assertEquals(2,source.reads.size)
        }finally{leave.complete(Unit);session.close()}
        assertTrue(source.closed);assertEquals(0L,session.state.value.retainedBytes)
    }
    @Test fun staleNativeCompletionNeverBecomesAVisibleFrame()=runBlocking {
        val source=Source();val backend=Renderer();val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        source.onRead={if(source.reads.size==1){entered.complete(Unit);withContext(NonCancellable){leave.await()}}}
        val session=session(this,source,backend,binding,1L,{_,_->true})
        try{
            session.request(view(0.0));withTimeout(3000){entered.await()};session.request(view(1.0));session.request(view(2.0))
            leave.complete(Unit);until{session.state.value.ready}
            assertEquals(listOf(0u,2u),source.reads.map{it.x});assertEquals(2.0,backend.frame?.view?.left)
            assertEquals(1,backend.next)
        }finally{leave.complete(Unit);session.close()}
    }
    @Test fun concurrentCancelledCloseJoinsInFlightReadAndAllImages()=runBlocking {
        val source=Source();val backend=Renderer();val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        source.onRead={entered.complete(Unit);withContext(NonCancellable){leave.await()}}
        val session=session(this,source,backend,binding,1L,{_,_->true})
        session.request(view(0.0));withTimeout(3000){entered.await()}
        val first=launch(start=CoroutineStart.UNDISPATCHED){session.close()}
        val second=launch(start=CoroutineStart.UNDISPATCHED){session.close()}
        try{first.cancel();assertFalse(first.isCompleted);assertFalse(second.isCompleted);assertFalse(source.closed)}
        finally{leave.complete(Unit);withContext(NonCancellable){first.join();second.join();session.close()}}
        assertTrue(source.closed);assertEquals(0,backend.next)
    }
    @Test fun visibleSetOverBudgetFailsBeforeAnyPixelRead()=runBlocking {
        val source=Source().apply{many=true};val backend=Renderer();val session=session(this,source,backend,binding,1L,{_,_->true},GiantViewportSession.TILE_CHARGE)
        try{session.request(view(0.0));until{session.state.value.message!=null};assertTrue(source.reads.isEmpty());assertNull(backend.frame)}finally{session.close()}
    }
    @Test fun changedSourceIdentityCannotReuseOldFrame()=runBlocking {
        val source=Source();val backend=Renderer();val session=session(this,source,backend,binding,1L,{_,_->true})
        try{
            session.request(view(0.0));until{session.state.value.ready};source.identity="c".repeat(64);session.request(view(1.0))
            until{session.state.value.message!=null};assertNull(backend.frame);assertEquals(1,source.reads.size);assertEquals(0L,session.state.value.retainedBytes)
        }finally{session.close()}
    }
    @Test fun imageCloseWaitsForCrossActorExportOwner()=runBlocking {
        val source=Source();val session=session(this,source,Renderer(),binding,1L,{_,_->true});val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        val export=launch(start=CoroutineStart.UNDISPATCHED){session.withExclusiveImage{entered.complete(Unit);withContext(NonCancellable){leave.await()}}}
        withTimeout(3000){entered.await()};val closing=launch(start=CoroutineStart.UNDISPATCHED){session.close()}
        try{export.cancel();assertFalse(source.closed);assertFalse(closing.isCompleted)}finally{leave.complete(Unit);withContext(NonCancellable){export.join();closing.join()}}
        assertTrue(source.closed)
    }
    @Test fun cancelledParentStillRetiresOwnedNativeHandle()=runBlocking {
        val parent=Job().apply{cancel()};val source=Source();val backend=Renderer()
        val session=session(CoroutineScope(coroutineContext+parent),source,backend,binding,1L,{_,_->true})
        session.close();assertTrue(source.closed);assertTrue(backend.events.contains("retire:end"))
    }
    @Test fun revisionOrAttachmentChangeDuringReadPreventsUploadAndPublication()=runBlocking {
        for (changed in listOf("sequence","hash","attachment")) {
            val source=Source();val backend=Renderer();val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
            var liveBinding=binding;var liveEpoch=1L
            source.onRead={entered.complete(Unit);withContext(NonCancellable){leave.await()}}
            val session=session(this,source,backend,binding,1L,{expected,epoch->expected==liveBinding&&epoch==liveEpoch})
            try {
                session.request(view(0.0));withTimeout(3000){entered.await()}
                when(changed) {
                    "sequence" -> liveBinding=binding.copy(hostSeq=2uL)
                    "hash" -> liveBinding=binding.copy(stateHash="d".repeat(64))
                    else -> liveEpoch=2L
                }
                leave.complete(Unit);until{!session.state.value.busy}
                assertNull(backend.frame);assertEquals(0,backend.next);assertEquals(0L,session.state.value.retainedBytes)
            } finally {leave.complete(Unit);session.close()}
        }
    }
    @Test fun newViewportHidesPreviousFrameWhileKeepingItsResourcesCharged()=runBlocking {
        val source=Source();val backend=Renderer();val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        val session=session(this,source,backend,binding,4L,{_,epoch->epoch==4L})
        try {
            session.request(view(0.0));until{session.state.value.ready}
            assertEquals(binding,backend.frame?.binding);assertEquals(4L,backend.frame?.attachmentEpoch)
            source.onRead={entered.complete(Unit);withContext(NonCancellable){leave.await()}}
            session.request(view(1.0));assertNull(backend.frame);withTimeout(3000){entered.await()}
            assertEquals(2*GiantViewportSession.TILE_CHARGE,session.state.value.retainedBytes)
            assertFalse(backend.events.any{it.startsWith("release:")})
            leave.complete(Unit);until{session.state.value.ready}
            assertEquals(2uL,backend.frame?.generation)
        } finally {leave.complete(Unit);session.close()}
    }
    @Test fun singularPhysicalTransformRefusesBeforeAnyTileRead()=runBlocking {
        val source=Source();val session=session(this,source,Renderer(),binding,1L,{_,_->true})
        try {
            assertTrue(runCatching{session.request(view(0.0).copy(a=1.0,b=2.0,c=2.0,d=4.0))}.exceptionOrNull() is IllegalArgumentException)
            assertTrue(source.reads.isEmpty());assertEquals(0uL,session.state.value.generation)
        } finally {session.close()}
    }
    @Test fun mismatchedTileDocumentGeometryRefusesBeforeForeignUpload()=runBlocking {
        val source=Source().apply{documentOffset=1u};val backend=Renderer()
        val session=session(this,source,backend,binding,1L,{_,_->true})
        try {
            session.request(view(0.0));until{session.state.value.message!=null}
            assertEquals(0,backend.next);assertNull(backend.frame);assertEquals(0L,session.state.value.retainedBytes)
        } finally {session.close()}
    }
    @Test fun failedRendererRetirementRetainsEveryImageAndItsCharge()=runBlocking {
        val source=Source();val backend=Renderer()
        val session=session(this,source,backend,binding,1L,{_,_->true},GiantViewportSession.TILE_CHARGE)
        session.request(view(0.0));until{session.state.value.ready}
        backend.onRetire={throw IllegalStateException("renderer still owns its recorded image")}
        session.request(view(1.0));until{source.closed}
        assertEquals(GiantViewportSession.TILE_CHARGE,session.state.value.retainedBytes)
        assertFalse(backend.events.any{it.startsWith("release:")})
        assertTrue(runCatching{session.close()}.isFailure)
    }
    @Test fun closeStillJoinsNativeReadWhenHidingTheBackendFails()=runBlocking {
        val source=Source();val backend=Renderer();val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        val session=session(this,source,backend,binding,1L,{_,_->true})
        session.request(view(0.0));until{session.state.value.ready}
        source.onRead={entered.complete(Unit);withContext(NonCancellable){leave.await()}}
        session.request(view(1.0));withTimeout(3000){entered.await()}
        backend.onPresent={throw IllegalStateException("renderer is still recorded")}
        val closing=async(start=CoroutineStart.UNDISPATCHED){runCatching{session.close()}}
        try {assertFalse(closing.isCompleted);assertFalse(source.closed)}
        finally {leave.complete(Unit)}
        assertTrue(closing.await().isFailure);assertTrue(source.closed)
        assertTrue(session.state.value.retainedBytes>=GiantViewportSession.TILE_CHARGE)
        assertFalse(backend.events.any{it.startsWith("release:")})
    }

    @Test fun sharedPoolExhaustionRefusesBeforeNativeReadEvenWithLocalCapacity()=runBlocking {
        val source=Source();val backend=Renderer();val resources=Resources(1u)
        val session=session(this,source,backend,binding,1L,{_,_->true},resources=resources)
        try { session.request(view(0.0));until{session.state.value.message!=null}
            assertTrue(source.reads.isEmpty());assertEquals(0,backend.next)
            assertEquals(0uL,resources.used);assertEquals(0L,session.state.value.retainedBytes)
        } finally {session.close()}
    }
    @Test fun failedReservationReleaseCannotDoubleReleasePlatformImageOrCreditItsBytes()=runBlocking {
        val source=Source();val backend=Renderer();val resources=Resources()
        val session=session(this,source,backend,binding,1L,{_,_->true},resources=resources)
        session.request(view(0.0));until{session.state.value.ready};resources.failClose=true
        assertTrue(runCatching{session.close()}.isFailure)
        assertEquals(GiantViewportSession.TILE_CHARGE.toULong(),resources.used)
        assertEquals(GiantViewportSession.TILE_CHARGE,session.state.value.retainedBytes)
        assertEquals(1,backend.events.count{it=="release:1"})
        // The failed owner remains retained. No second close claims recovery.
        assertTrue(runCatching{session.close()}.isFailure)
        assertEquals(1,backend.events.count{it=="release:1"})
    }
    @Test fun globalChargeOutlivesFailedRendererRetirementAndNativeClose()=runBlocking {
        val source=Source();val backend=Renderer();val resources=Resources()
        val session=session(this,source,backend,binding,1L,{_,_->true},resources=resources)
        session.request(view(0.0));until{session.state.value.ready}
        backend.onRetire={throw IllegalStateException("pipeline retained")}
        assertTrue(runCatching{session.close()}.isFailure)
        assertTrue(source.closed);assertEquals(0,resources.closes)
        assertEquals(GiantViewportSession.TILE_CHARGE.toULong(),resources.used)
    }

    @Test fun reentrantUiPublicationOfSameViewDoesNotRecursivelyRestartRequest()=runBlocking {
        val source=Source();val backend=Renderer()
        val session=session(this,source,backend,binding,1L,{_,_->true})
        var publications=0;var closing=false
        backend.onPresent={if(!closing){check(++publications<10);session.request(view(0.0))}}
        try {session.request(view(0.0));until{session.state.value.ready}
            assertEquals(1,source.reads.size);assertTrue(publications<=2)
        }finally{closing=true;session.close()}
    }

    @Test fun immutableContentModeDispatchesOnlyTheSelectedTileReaderAndForwardsColorConsent()=runBlocking {
        for (consent in listOf(false, true)) {
            val image=EditingSource();val backend=Renderer()
            val owner=session(this,image,backend,binding,1L,{_,_->true},
                content=GiantTileContent.Composed,assumeUntaggedSrgb=consent)
            try {
                owner.request(view(0.0));until{owner.state.value.ready}
                assertEquals(listOf(GiantTileAddress(0u,0u,0u) to consent),image.edits)
                assertTrue(image.source.reads.isEmpty())
                assertEquals(GiantTileContent.Composed,backend.frame?.content)
            } finally {owner.close()}
        }
        val image=EditingSource();val backend=Renderer()
        val sourceOwner=session(this,image,backend,binding,1L,{_,_->true})
        try {
            sourceOwner.request(view(0.0));until{sourceOwner.state.value.ready}
            assertEquals(1,image.source.reads.size);assertTrue(image.edits.isEmpty())
            assertEquals(GiantTileContent.Source,backend.frame?.content)
        } finally {sourceOwner.close()}
    }

    @Test fun missingCompositionCapabilityAndUnsupportedLevelRefuseBeforeTileAdmission()=runBlocking {
        val plain=Source();val backend=Renderer();val resources=Resources()
        val owner=session(this,plain,backend,binding,1L,{_,_->true},resources=resources,
            content=GiantTileContent.Composed)
        try {
            owner.request(view(0.0));until{owner.state.value.message!=null}
            assertTrue(owner.state.value.message.orEmpty().contains("composition is unavailable"))
            assertEquals(0,plain.viewportCalls);assertTrue(plain.reads.isEmpty())
            assertEquals(0,backend.next);assertNull(backend.frame);assertEquals(0uL,resources.used)
        } finally {owner.close()}
        val image=EditingSource(Source().apply{level=3u});val second=Renderer()
        val limited=session(this,image,second,binding,1L,{_,_->true},content=GiantTileContent.Composed)
        try {
            limited.request(view(0.0));until{limited.state.value.message!=null}
            assertTrue(limited.state.value.message.orEmpty().contains("levels 0–2"))
            assertEquals(1,image.source.viewportCalls);assertTrue(image.edits.isEmpty())
            assertEquals(0,second.next);assertNull(second.frame)
            assertEquals(0L,limited.state.value.retainedBytes)
        } finally {limited.close()}
    }

    @Test fun staleCommittedRevisionDuringComposedReadCannotPublishOrUpload()=runBlocking {
        val image=EditingSource();val backend=Renderer();val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        var liveBinding=binding
        image.onEdit={entered.complete(Unit);withContext(NonCancellable){leave.await()}}
        val owner=session(this,image,backend,binding,1L,{expected,_->expected==liveBinding},
            content=GiantTileContent.Composed)
        try {
            owner.request(view(0.0));withTimeout(3000){entered.await()}
            liveBinding=binding.copy(stateHash="d".repeat(64))
            leave.complete(Unit);until{!owner.state.value.busy}
            assertEquals(0,backend.next);assertNull(backend.frame)
            assertEquals(0L,owner.state.value.retainedBytes)
        } finally {leave.complete(Unit);owner.close()}
    }

    @Test fun composedReadCancellationJoinsAndRetiresItsChargedReservation()=runBlocking {
        val image=EditingSource();val backend=Renderer();val resources=Resources()
        val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        image.onEdit={entered.complete(Unit);withContext(NonCancellable){leave.await()}}
        val owner=session(this,image,backend,binding,1L,{_,_->true},resources=resources,
            content=GiantTileContent.Composed)
        owner.request(view(0.0));withTimeout(3000){entered.await()}
        val closing=launch(start=CoroutineStart.UNDISPATCHED){owner.close()}
        try {
            assertFalse(closing.isCompleted)
            assertEquals(GiantViewportSession.TILE_CHARGE,owner.state.value.retainedBytes)
        } finally {leave.complete(Unit);closing.join()}
        assertTrue(image.source.closed);assertNull(backend.frame);assertEquals(0,backend.next)
        assertEquals(0uL,resources.used);assertEquals(0L,owner.state.value.retainedBytes)
    }

    @Test fun composedImageChargeWaitsForRendererRetirementAcknowledgment()=runBlocking {
        val image=EditingSource();val backend=Renderer();val resources=Resources()
        val owner=session(this,image,backend,binding,1L,{_,_->true},
            max=GiantViewportSession.TILE_CHARGE,resources=resources,content=GiantTileContent.Composed)
        val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        try {
            owner.request(view(0.0));until{owner.state.value.ready}
            backend.onRetire={entered.complete(Unit);leave.await()}
            owner.request(view(1.0));withTimeout(3000){entered.await()}
            assertEquals(GiantViewportSession.TILE_CHARGE.toULong(),resources.used)
            assertFalse(backend.events.any{it=="release:1"})
            leave.complete(Unit);until{owner.state.value.ready}
            assertEquals(listOf(0u,1u),image.edits.map{it.first.x})
            assertTrue(backend.events.indexOf("retire:end")<backend.events.indexOf("release:1"))
        } finally {leave.complete(Unit);owner.close()}
        assertEquals(0uL,resources.used)
    }

    @Test fun sourceToComposedReplacementRetiresTheOldOwnerBeforeTheNewFrame()=runBlocking {
        val resources=Resources();val oldImage=EditingSource();val oldBackend=Renderer()
        val old=session(this,oldImage,oldBackend,binding,1L,{_,_->true},resources=resources)
        old.request(view(0.0));until{old.state.value.ready}
        assertEquals(GiantTileContent.Source,oldBackend.frame?.content)
        old.close()
        assertTrue(oldImage.source.closed)
        assertTrue(oldBackend.events.indexOf("retire:end")<oldBackend.events.indexOf("release:1"))
        assertEquals(0uL,resources.used)
        val nextImage=EditingSource();val nextBackend=Renderer()
        val next=session(this,nextImage,nextBackend,binding.copy(hostSeq=2uL),2L,{_,_->true},
            resources=resources,content=GiantTileContent.Composed)
        try {
            next.request(view(0.0));until{next.state.value.ready}
            assertEquals(GiantTileContent.Composed,nextBackend.frame?.content)
            assertTrue(nextImage.source.reads.isEmpty());assertEquals(1,nextImage.edits.size)
            assertEquals(GiantViewportSession.TILE_CHARGE.toULong(),resources.used)
        } finally {next.close()}
        assertEquals(0uL,resources.used)
    }

}
