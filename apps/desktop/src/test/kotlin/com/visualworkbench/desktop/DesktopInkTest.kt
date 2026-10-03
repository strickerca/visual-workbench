@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

class DesktopInkTest {
    @Test fun onlyAcceptedRawSamplesStreamAndDurableCommitPrecedesSuccessfulClose() = runBlocking {
        val order = mutableListOf<String>()
        val native = InkFixture(order)
        val link = object: LinkFixture() {
            override fun streamStroke(stroke: WorkbenchStroke, batch: SampleBatch, firstSample: UInt) {
                assertEquals(0u, firstSample); assertSame(native, stroke)
                super.streamStroke(stroke,batch,firstSample);order += "stream"
            }
        }
        val run = inkRun(link)
        run.offer(DesktopSample(Point(12.0,13.0),1020, .4f)); run.finish()
        val work = launch { withTimeout(2000) { run.execute(InkProject(native), {}, { order += "durable" }, { order += "finish:$it" }) } }
        native.entered.await()
        assertTrue(link.streamed.isEmpty()); assertEquals(0,native.commits)
        native.release.complete(Unit);work.join()
        assertEquals(listOf("append","stream","commit","durable","finish:false","dispose"),order)
        val sent=link.streamed.single()
        assertArrayEquals(doubleArrayOf(10.0,12.0),sent.x,0.0)
        assertArrayEquals(floatArrayOf(1f,.4f),sent.pressure,0f)
        assertTrue(sent.timeMs.contentEquals(uintArrayOf(0u,20u)))
        assertTrue(run.committed);assertEquals(0,native.predictions)
    }

    @Test fun acceptedReceiptIsPublishedBeforeAThrowingPreviewClose() = runBlocking {
        for (failure in listOf(SessionFailureKind.Timeout, SessionFailureKind.Authentication)) {
            val order = mutableListOf<String>(); val native = InkFixture(order).apply { release.complete(Unit) }
            val run = inkRun(null); run.finish()
            val receipts = mutableListOf<ProjectInfo>()
            val thrown = runCatching { run.execute(InkProject(native), {}, { receipts += it; yield() }, { cancel ->
                assertFalse(cancel); throw SessionFailure(failure)
            }) }.exceptionOrNull()
            assertEquals(failure, (thrown as? SessionFailure)?.kind)
            assertEquals(listOf(inkInfo), receipts); assertTrue(run.committed)
            assertEquals(1, native.commits); assertFalse(order.contains("cancel")); assertEquals("dispose", order.last())
        }
    }

    @Test fun cancellationDuringPreviewCloseCannotSkipTheAcceptedReceipt() = runBlocking {
        val order = mutableListOf<String>(); val native = InkFixture(order).apply { release.complete(Unit) }
        val run = inkRun(null); run.finish()
        val closing = CompletableDeferred<Unit>(); val receipts = mutableListOf<ProjectInfo>()
        val work = launch { run.execute(InkProject(native), {}, { receipts += it; yield() }, { cancel ->
            assertFalse(cancel); closing.complete(Unit); awaitCancellation()
        }) }
        withTimeout(2000) { closing.await() }
        work.cancelAndJoin()
        assertEquals(listOf(inkInfo), receipts); assertTrue(run.committed)
        assertEquals(1, native.commits); assertFalse(order.contains("cancel")); assertEquals("dispose", order.last())
    }

    @Test fun cancelDuringAppendNeverStreamsTheReturnedBatchOrCommits() = runBlocking {
        val order=mutableListOf<String>();val native=InkFixture(order);val link=LinkFixture();val run=inkRun(link)
        val work=launch{run.execute(InkProject(native),{fail("cancelled ink published")},{fail("cancelled commit")},{link.finishPreview("gesture",it)})}
        native.entered.await();run.cancel();native.release.complete(Unit);work.join()
        assertTrue(link.streamed.isEmpty());assertEquals(0,native.commits)
        assertEquals(listOf("gesture" to true),link.finished)
        assertEquals("dispose",order.last())
    }

    @Test fun rejectedAppendCannotProduceAPeerPreview() = runBlocking {
        val order=mutableListOf<String>();val native=InkFixture(order).apply{reject=true;release.complete(Unit)}
        val link=LinkFixture();val run=inkRun(link);run.finish()
        runCatching{run.execute(InkProject(native),{fail("rejected geometry")},{fail("rejected commit")},{link.finishPreview("gesture",it)})}.onSuccess{fail("append rejection swallowed")}
        assertTrue(link.streamed.isEmpty());assertEquals(0,native.commits)
        assertEquals(listOf("gesture" to true),link.finished)
    }

    @Test fun fullInputQueueCancelsTheWholeGestureInsteadOfDroppingSamples() {
        val run=inkRun(null)
        repeat(511){assertTrue(run.offer(DesktopSample(Point(it.toDouble(),0.0),1001L+it)))}
        assertFalse(run.offer(DesktopSample(Point(999.0,0.0),2000)))
        assertTrue(run.cancelled);assertFalse(run.finished)
    }

    @Test fun geometryAdditionRejectsMalformedEndsAndPreservesIndependentKnownCoordinates() {
        val before=Contours(longArrayOf(0,256,0),longArrayOf(0,0,256),uintArrayOf(3u))
        val added=Contours(longArrayOf(512,768,512),longArrayOf(0,0,256),uintArrayOf(3u))
        val joined=appendContours(before,added)
        assertTrue(joined.ends.contentEquals(uintArrayOf(3u,6u)))
        assertTrue(joined.x.contentEquals(longArrayOf(0,256,0,512,768,512)))
        for(invalid in listOf(uintArrayOf(2u),uintArrayOf(3u,2u),uintArrayOf(4u)))
            runCatching{appendContours(before,added.copy(ends=invalid))}.onSuccess{fail("malformed geometry admitted")}
    }

    @Test fun sameKindShapeDragPreservesDocumentSpaceWidthAndIdentity() {
        val link=LinkFixture();var time=0L
        val lease=NewObjectPreviewLease("gesture",LiveAttachment(1,link),"object","document","layer",1000,
            Shape.Rectangle(Rect(0.0,0.0,1.0,1.0)),ObjectStyle(0xffffffffu,3.0),{time})
        val first=checkNotNull(draftShape(Tool.Rectangle,Point(30.0,40.0),Point(10.0,20.0)))
        lease.update(first);time=9_000_000
        val second=checkNotNull(draftShape(Tool.Rectangle,Point(30.0,40.0),Point(80.0,120.0)))
        lease.update(second)
        assertEquals(listOf(first,second),link.newObjects.map{it.shape})
        assertTrue(link.newObjects.all{it.transform==Transform()&&!it.style.screenConstantWidth&&it.style.width==3.0&&it.gestureId=="gesture"&&it.objectId=="object"})
        assertEquals(Rect(10.0,20.0,20.0,20.0),(first as Shape.Rectangle).rectangle)
    }

    @Test fun liveObjectUpdatesAreBoundedTo120HzWithoutChangingTheFinalData() {
        val link=LinkFixture();var time=0L
        val lease=ObjectPreviewLease("gesture",LiveAttachment(1,link),"document","object",PreviewKind.HandleDrag,{time})
        lease.update(Transform(e=1.0),ObjectStyle(0xffffffffu,3.0))
        time=1_000_000;lease.update(Transform(e=2.0),ObjectStyle(0xffffffffu,3.0))
        time=8_333_334;lease.update(Transform(e=3.0),ObjectStyle(0xffffffffu,3.0))
        assertEquals(listOf(1u,2u),link.objects.map{it.sequence})
        assertEquals(listOf(1.0,3.0),link.objects.map{it.transform.e})
    }
}

private fun inkRun(link:ProjectLink?):DesktopInkRun = DesktopInkRun(
    StrokeOptions("gesture","transaction","object","document","layer","installed",1u,1000,"pen",3.0,0xffffffffu),
    RenderBinding(1,"project","document",0u,"hash"),Camera(Point(0.0,0.0),1.0,0.0,100.0,100.0),
    LayerInfo("layer","Annotations",true,false,1.0,"normal"),link?.let{LiveAttachment(1,it)},DesktopSample(Point(10.0,10.0),1000))

private val inkInfo=ProjectInfo("project","fixture","installed",1u,false,false,0u,"hash",listOf("document"))
private class InkFixture(private val order:MutableList<String>):WorkbenchStroke {
    val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
    var reject=false;var commits=0;var predictions=0;private var samples=0uL;private var polygons=0uL
    override suspend fun append(batch:SampleBatch):InkUpdate{
        entered.complete(Unit);release.await();order+="append"
        if(reject)throw CoreFailure(CoreFailureKind.Invalid)
        samples+=batch.x.size.toULong()
        val first=polygons++;return InkUpdate(batch.sequence,first,samples,Contours(longArrayOf(0,256,0),longArrayOf(0,0,256),uintArrayOf(3u)))
    }
    override suspend fun predict(batch:SampleBatch):InkUpdate{predictions++;error("Prediction must never be requested")}
    override suspend fun commit():ProjectInfo{commits++;order+="commit";return inkInfo}
    override fun cancel(){order+="cancel"}
    override suspend fun dispose(){order+="dispose"}
}
private class InkProject(private val native:WorkbenchStroke):WorkbenchProject {
    override val changes=emptyFlow<ProjectChange>()
    override suspend fun info()=inkInfo
    override suspend fun beginStroke(options:StrokeOptions)=native
    override suspend fun document(documentId:String):DocumentSnapshot=error("unused")
    override suspend fun background(documentId:String,memoryBudgetBytes:ULong,assumeUntaggedSrgb:Boolean):BackgroundImage=error("unused")
    override suspend fun edit(options:EditOptions,commands:List<EditCommand>):ProjectInfo=error("unused")
    override suspend fun undoRedo(options:EditOptions,redo:Boolean):ProjectInfo=error("unused")
    override suspend fun render(documentId:String,rectangle:Rect?):RenderList=error("unused")
    override suspend fun export(options:ExportOptions):ExportResult=error("unused")
    override suspend fun close()=Unit
}
