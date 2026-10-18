package com.visualworkbench.shared

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class GiantViewportSessionTest {
    private fun view(x: Double) = GiantViewport(x,0.0,x+256.0,256.0,1.0,0.0,0.0,1.0)
    private class Source : GiantImage {
        val reads = mutableListOf<GiantTileAddress>()
        var identity = "a".repeat(64)
        var onRead: suspend () -> Unit = {}
        var closed = false
        var many = false
        override suspend fun viewport(view: GiantViewport): GiantViewportPlan {
            val keys = if(many)(0u..2u).map{GiantTileAddress(0u,it,0u)} else listOf(GiantTileAddress(0u,view.left.toUInt(),0u))
            return GiantViewportPlan(identity,"b".repeat(64),0u,keys,emptyList())
        }
        override suspend fun tile(address: GiantTileAddress): GiantTilePixels {
            reads += address; onRead()
            return GiantTilePixels(identity,"b".repeat(64),address,address.x*256u,0u,256u,256u,256u,256u,ByteArray(256*256*4){if(it%4==3)255.toByte() else 0})
        }
        override suspend fun exportPng(options: GiantPngOptions): GiantPngReceipt = error("not used")
        override suspend fun close(){closed=true}
    }
    private class Renderer : GiantRenderBackend<Int> {
        val events = mutableListOf<String>(); var next=0; var frame:GiantVisibleFrame<Int>?=null
        var onRetire:suspend()->Unit={}
        override suspend fun create(pixels:GiantTilePixels):Int=(++next).also{events+="create:$it"}
        override fun present(frame:GiantVisibleFrame<Int>?){this.frame=frame;events+="present:${frame?.generation}"}
        override suspend fun retire(){events+="retire:start";onRetire();events+="retire:end"}
        override fun release(image:Int){events+="release:$image"}
    }
    private suspend fun until(test:()->Boolean){withTimeout(3000){while(!test())yield()}}
    @Test fun evictionCreditsOnlyAfterActualRendererRetirement()=runBlocking {
        val source=Source();val backend=Renderer();val session=GiantViewportSession(this,source,backend,GiantViewportSession.TILE_CHARGE)
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
        val session=GiantViewportSession(this,source,backend)
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
        val session=GiantViewportSession(this,source,backend)
        session.request(view(0.0));withTimeout(3000){entered.await()}
        val first=launch(start=CoroutineStart.UNDISPATCHED){session.close()}
        val second=launch(start=CoroutineStart.UNDISPATCHED){session.close()}
        try{first.cancel();assertFalse(first.isCompleted);assertFalse(second.isCompleted);assertFalse(source.closed)}
        finally{leave.complete(Unit);withContext(NonCancellable){first.join();second.join();session.close()}}
        assertTrue(source.closed);assertEquals(0,backend.next)
    }
    @Test fun visibleSetOverBudgetFailsBeforeAnyPixelRead()=runBlocking {
        val source=Source().apply{many=true};val backend=Renderer();val session=GiantViewportSession(this,source,backend,GiantViewportSession.TILE_CHARGE)
        try{session.request(view(0.0));until{session.state.value.message!=null};assertTrue(source.reads.isEmpty());assertNull(backend.frame)}finally{session.close()}
    }
    @Test fun changedSourceIdentityCannotReuseOldFrame()=runBlocking {
        val source=Source();val backend=Renderer();val session=GiantViewportSession(this,source,backend)
        try{
            session.request(view(0.0));until{session.state.value.ready};source.identity="c".repeat(64);session.request(view(1.0))
            until{session.state.value.message!=null};assertNull(backend.frame);assertEquals(1,source.reads.size);assertEquals(0L,session.state.value.retainedBytes)
        }finally{session.close()}
    }
    @Test fun imageCloseWaitsForCrossActorExportOwner()=runBlocking {
        val source=Source();val session=GiantViewportSession(this,source,Renderer());val entered=CompletableDeferred<Unit>();val leave=CompletableDeferred<Unit>()
        val export=launch(start=CoroutineStart.UNDISPATCHED){session.withExclusiveImage{entered.complete(Unit);withContext(NonCancellable){leave.await()}}}
        withTimeout(3000){entered.await()};val closing=launch(start=CoroutineStart.UNDISPATCHED){session.close()}
        try{export.cancel();assertFalse(source.closed);assertFalse(closing.isCompleted)}finally{leave.complete(Unit);withContext(NonCancellable){export.join();closing.join()}}
        assertTrue(source.closed)
    }
    @Test fun cancelledParentStillRetiresOwnedNativeHandle()=runBlocking {
        val parent=Job().apply{cancel()};val source=Source();val backend=Renderer()
        val session=GiantViewportSession(CoroutineScope(coroutineContext+parent),source,backend)
        session.close();assertTrue(source.closed);assertTrue(backend.events.contains("retire:end"))
    }
}
