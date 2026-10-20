package com.visualworkbench.desktop

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test
import java.lang.reflect.Proxy

class DesktopStreamPublicationTest {
    private val selection=CaptureLocalSelection.Monitor(CaptureLocalMonitor(7u,"synthetic-display",CaptureLocalRect(-400,200,320u,240u),144u,0u,true,1000u))
    private val source=selection.publicationSource("019a5d54-0000-7000-8000-000000000001","Synthetic source")
    private val options=CaptureLocalOptions(true,selection,"unused-fixture-helper","0".repeat(64))
    private fun link():ProjectLink=Proxy.newProxyInstance(ProjectLink::class.java.classLoader,arrayOf(ProjectLink::class.java)){_,_,_->error("No native link method belongs in this fixture") } as ProjectLink
    private class Publisher:WorkbenchCapturePublisher {
        var closes=0
        override val signals=emptyFlow<CapturePublicationSignal>()
        override fun signal()=CapturePublicationSignal(1u,3u,5u,true,false,1u,null,null)
        override suspend fun publishFrame(encoded:ByteArray)=error("Pump owns native pixels")
        override suspend fun provideOriginal(requestSequence:ULong,identity:CaptureImportDescriptor,encoded:ByteArray)=error("Pump owns native pixels")
        override suspend fun originalUnavailable(requestSequence:ULong)=error("Pump owns native pixels")
        override suspend fun close(){closes++}
    }
    private class Producer:WorkbenchLocalCapture {
        var closes=0;var cancellations=0;var failClose=false
        override fun cancel(){cancellations++}
        override suspend fun pumpOnce():CaptureLocalProgress {awaitCancellation()}
        override suspend fun close(){closes++;if(failClose)error("retained close")}
    }
    @Test fun selectedSourcePreservesExactPhysicalIdentity(){
        assertEquals("monitor",source.sourceKind);assertEquals(-400,source.hostX);assertEquals(200,source.hostY)
        assertEquals(320u,source.width);assertEquals(240u,source.height);assertEquals(1.5,source.dpiScale,0.0)
        assertEquals("synthetic-display",source.monitorId);assertEquals(1000L,source.observedTimestampNs)
        assertEquals(15u,source.targetFps);assertTrue(source.explicitOwnerAction)
    }
    @Test fun closeCancelsAndJoinsLateAcquisitionBeforeRetiringPublisher()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val entered=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>()
        val publisher=Publisher();val producer=Producer()
        val owner=DesktopStreamPublication(scope,{},openPublisher={_,_->publisher},openProducer={_,_->withContext(NonCancellable){entered.complete(Unit);release.await();producer}})
        val opening=async(start=CoroutineStart.UNDISPATCHED){runCatching{owner.start(link(),source,options){true}}}
        entered.await()
        val closing=launch(start=CoroutineStart.UNDISPATCHED){owner.close()}
        yield();assertFalse(closing.isCompleted);assertEquals(0,publisher.closes)
        release.complete(Unit);closing.join();opening.await()
        assertEquals(1,producer.closes);assertEquals(1,publisher.closes);assertFalse(owner.state.value.active)
        scope.cancel()
    }
    @Test fun failedProducerCloseRetainsPublisherUntilExactRetry()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val publisher=Publisher();val producer=Producer()
        val owner=DesktopStreamPublication(scope,{},openPublisher={_,_->publisher},openProducer={_,_->producer})
        owner.start(link(),source,options){true};producer.failClose=true
        assertTrue(runCatching{owner.close()}.isFailure);assertTrue(owner.state.value.cleanupPending)
        assertEquals(0,publisher.closes);assertEquals(1,producer.closes)
        producer.failClose=false;owner.close()
        assertEquals(2,producer.closes);assertEquals(1,publisher.closes)
        owner.close();assertEquals(1,publisher.closes);scope.cancel()
    }
    @Test fun changedAttachmentDuringAcquisitionCannotStartThePump()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val publisher=Publisher();var current=true;var producerOpens=0
        val owner=DesktopStreamPublication(scope,{},openPublisher={_,_->current=false;publisher},openProducer={_,_->producerOpens++;Producer()})
        assertTrue(runCatching{owner.start(link(),source,options){current}}.isFailure)
        assertEquals(0,producerOpens);assertEquals(1,publisher.closes);assertFalse(owner.state.value.active)
        scope.cancel()
    }
    @Test fun retainedSelectionCannotMoveToReconnectedEpochOnTheSameAttachment()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val link=LinkFixture().also{it.initialStatus=sessionStatus(1u,SyncStatus.Synced,SessionCarrier.QuicTether)}
        val attached=LiveAttachment(1,link);var nativeEpoch=3uL
        val chosen=LiveCaptureConnectionFence(attached){nativeEpoch}
        assertTrue(chosen.current())
        attached.observe(sessionStatus(2u,SyncStatus.Reconnecting,SessionCarrier.QuicTether))
        nativeEpoch=5u;attached.observe(sessionStatus(3u,SyncStatus.Synced,SessionCarrier.QuicTether))
        var opens=0
        val owner=DesktopStreamPublication(scope,{},openPublisher={_,_->opens++;Publisher()})
        assertTrue(runCatching{owner.start(link,source,options,chosen::current)}.isFailure)
        assertEquals(0,opens);assertFalse(owner.state.value.active)
        // Native epoch also fences a reconnect hidden by status conflation.
        val fresh=LiveCaptureConnectionFence(attached){nativeEpoch};assertTrue(fresh.current())
        nativeEpoch=7u;assertFalse(fresh.current());scope.cancel()
    }
    @Test fun reconnectDuringPublisherAcquisitionRetiresItBeforeOpeningThePixelProducer()=runBlocking {
        val scope=CoroutineScope(SupervisorJob()+Dispatchers.Unconfined)
        val link=LinkFixture().also{it.initialStatus=sessionStatus(1u,SyncStatus.Synced,SessionCarrier.QuicTether)}
        val attached=LiveAttachment(1,link);var nativeEpoch=3uL
        val chosen=LiveCaptureConnectionFence(attached){nativeEpoch};val publisher=Publisher();var producers=0
        val owner=DesktopStreamPublication(scope,{},openPublisher={_,_->nativeEpoch=5u;publisher},openProducer={_,_->producers++;Producer()})
        assertTrue(runCatching{owner.start(link,source,options,chosen::current)}.isFailure)
        assertEquals(0,producers);assertEquals(1,publisher.closes);scope.cancel()
    }
}
