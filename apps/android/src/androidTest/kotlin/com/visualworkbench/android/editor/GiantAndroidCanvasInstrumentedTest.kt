package com.visualworkbench.android.editor

import android.graphics.Bitmap
import android.graphics.Canvas
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewGroup
import androidx.graphics.lowlatency.CanvasFrontBufferedRenderer
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.visualworkbench.android.MainActivity
import com.visualworkbench.shared.*
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/** Synthetic fixtures, not product/RSS/physical-pen acceptance. Do not execute
 * without the separate device/build grant. Only the framework callback test creates a real
 * framework surface; fake retirement callbacks do not establish GPU retirement. */
@RunWith(AndroidJUnit4::class)
class GiantAndroidCanvasInstrumentedTest {
    @Test fun rgbaBytesUseOpaqueSrgbAndActualStorageFitsTheTileReservation() = runBlocking {
        val factory = GiantAndroidBitmapFactory()
        val bitmap = factory.create(tile(2, 1, byteArrayOf(-1,1,2,-1,3,4,5,-1)))
        try {
            assertEquals(0xffff0102.toInt(),bitmap.getPixel(0,0)); assertEquals(0xff030405.toInt(),bitmap.getPixel(1,0))
            assertFalse(bitmap.hasAlpha()); assertTrue(bitmap.colorSpace?.isSrgb == true)
            admitGiantAndroidBitmap(2,1,8,bitmap.rowBytes,bitmap.allocationByteCount)
        } finally { releaseGiantAndroidBitmap(bitmap) } // Never passed to graphics.
    }
    @Test fun invalidAlphaAndDimensionsRefuseBeforeAnImageCanBePresented() = runBlocking {
        val factory = GiantAndroidBitmapFactory()
        for (bad in listOf(tile(1,1,byteArrayOf(1,2,3,0)),tile(257,1,ByteArray(1028)),tile(2,1,ByteArray(4)))) {
            try { factory.create(bad); fail("invalid image admitted") } catch (_: IllegalArgumentException) { }
        }
        factory.releaseFailedAfterRetirement() // No Canvas/renderer ever existed.
    }
    @Test fun softwareDrawUsesOnlyDocumentRectanglesAndTheSuppliedCanvasTransform() = runBlocking {
        val factory = GiantAndroidBitmapFactory()
        val bitmap = factory.create(tile(2,1,byteArrayOf(-1,0,0,-1,0,-1,0,-1)))
        val target = Bitmap.createBitmap(8,4,Bitmap.Config.ARGB_8888)
        try {
            val descriptor = GiantTileDescriptor(GiantTileAddress(1u,0u,0u),0u,0u,4u,2u,2u,1u)
            val frame = GiantVisibleFrame(binding,1L,1uL,view,"a".repeat(64),"b".repeat(64),listOf(GiantRenderedTile(descriptor,bitmap)))
            val canvas = Canvas(target); canvas.translate(2f,1f)
            drawGiantAndroidTiles(canvas,frame)
            assertEquals(0,target.getPixel(1,1)); assertEquals(0xffff0000.toInt(),target.getPixel(2,1))
            assertEquals(0xffff0000.toInt(),target.getPixel(3,1)); assertEquals(0xff00ff00.toInt(),target.getPixel(4,1))
            assertEquals(0xff00ff00.toInt(),target.getPixel(5,1)); assertEquals(0,target.getPixel(6,1))
        } finally { releaseGiantAndroidBitmap(bitmap); target.recycle() } // Synchronous software draw only.
    }
    @Test fun closeSealsBeforeReleaseAndKeepsBitmapAndBothReservationsUntilAck() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source(); val pool = Resources(); val entered = CompletableDeferred<Unit>(); val ack = CompletableDeferred<Unit>()
        val owner = GiantAndroidCanvas.create(parent,source,pool,binding,1L,{_,_->true},{entered.complete(Unit);ack.await()},{})
        var closing:Deferred<Unit>? = null
        try {
            withContext(Dispatchers.Main.immediate) { owner.update(Core(),camera,PrecisionFrame()) }
            until { owner.status.ready }
            val bitmap = checkNotNull(owner.frame).tiles.single().image
            closing = async { owner.close() }
            withTimeout(3000) { entered.await() }
            assertFalse(owner.surfaceReserved); assertNull(owner.frame); assertFalse(owner.status.ready)
            assertFalse(bitmap.isRecycled)
            assertEquals(GIANT_ANDROID_SURFACE_BYTES+GiantViewportSession.TILE_CHARGE.toULong(),pool.used)
            ack.complete(Unit); closing.await()
            assertTrue(bitmap.isRecycled); assertTrue(source.closed); assertEquals(0uL,pool.used)
            owner.close(); assertEquals(2,pool.released)
        } finally {
            ack.complete(Unit)
            withContext(NonCancellable) { closing?.await(); owner.close() }; parent.cancel()
        }
    }
    @Test fun fullBindingEpochViewportAndCurrentOwnerFenceEveryFrame() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val current = AtomicBoolean(true); val pool = Resources()
        val owner = GiantAndroidCanvas.create(parent,Source(),pool,binding,1L,{_,_->current.get()},{},{})
        try {
            withContext(Dispatchers.Main.immediate) { owner.update(Core(),camera,PrecisionFrame()) }
            until { owner.status.ready }
            val old = checkNotNull(owner.matchingFrame(binding,1L,view))
            assertNull(owner.matchingFrame(binding.copy(hostSeq=2uL),1L,view))
            assertNull(owner.matchingFrame(binding,2L,view))
            assertNull(owner.matchingFrame(binding,1L,view.copy(a=2.0)))
            current.set(false); assertNull(owner.matchingFrame(binding,1L,view)); current.set(true)
            withContext(Dispatchers.Main.immediate) { owner.update(Core(),camera.copy(center=Point(2.0,0.0)),PrecisionFrame()) }
            assertNull(owner.matchingFrame(binding,1L,old.view))
            until { owner.status.ready }
            assertTrue(checkNotNull(owner.frame).generation>old.generation)
        } finally { withContext(NonCancellable) { owner.close() }; parent.cancel() }
    }
    @Test fun composedModeUsesEditingTilesAndRefusesASourceFrameAtDraw() = runBlocking {
        val parent=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source=Source();val pool=Resources();var editingCalls=0;var approved=false
        val editing=object:GiantImage by source,GiantEditingImage {
            override suspend fun editingTile(address:GiantTileAddress,foreign:SessionResourceCharge,
                assumeUntaggedSrgb:Boolean):GiantTilePixels {
                editingCalls++;approved=assumeUntaggedSrgb;return tile()
            }
        }
        val owner=GiantAndroidCanvas.create(parent,editing,pool,binding,1L,{_,_->true},{},{},
            content=GiantTileContent.Composed,assumeUntaggedSrgb=true)
        try {
            withContext(Dispatchers.Main.immediate){owner.update(Core(),camera,PrecisionFrame())}
            until{owner.status.ready}
            val frame=checkNotNull(owner.matchingFrame(binding,1L,view,GiantTileContent.Composed))
            assertEquals(GiantTileContent.Composed,frame.content)
            assertNull(owner.matchingFrame(binding,1L,view,GiantTileContent.Source))
            assertEquals(1,editingCalls);assertTrue(approved)
        } finally {withContext(NonCancellable){owner.close()};parent.cancel()}
        assertTrue(source.closed);assertEquals(0uL,pool.used)
    }
    @Test fun failedFactoryCleanupReturnsTheExactOwnerForRootQuarantine() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source().apply { failClose=true }
        val pool = Resources().apply { refuse=true }
        var held:GiantAndroidCanvas? = null
        try {
            try { GiantAndroidCanvas.create(parent,source,pool,binding,1L,{_,_->true},{fail("no renderer allocated")},{})
                fail("failed cleanup discarded")
            } catch (failure:GiantAndroidCreationFailure) { held=failure.owner }
            assertFalse(source.closed); assertEquals(0uL,pool.used)
            source.failClose=false; checkNotNull(held).close(); assertTrue(source.closed)
        } finally { source.failClose=false; withContext(NonCancellable) { held?.close() }; parent.cancel() }
    }
    @Test fun cancelledFactorySettlesLateSurfaceReservationBeforeReturningCancellation() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source(); val pool = Resources()
        val entered = CompletableDeferred<Unit>(); val proceed = CompletableDeferred<Unit>()
        val resources = object:SessionResources {
            override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge {
                val charge=pool.reserveForeign(bytes);entered.complete(Unit);proceed.await();return charge
            }
        }
        val creation = async { GiantAndroidCanvas.create(parent,source,resources,binding,1L,{_,_->true},{},{}) }
        try {
            withTimeout(3000) { entered.await() }; creation.cancel()
            assertFalse(source.closed); assertEquals(GIANT_ANDROID_SURFACE_BYTES,pool.used)
            proceed.complete(Unit)
            try { creation.await(); fail("cancelled factory adopted") } catch (_: CancellationException) { }
            assertTrue(source.closed); assertEquals(0uL,pool.used)
        } finally { proceed.complete(Unit);withContext(NonCancellable){creation.join()};parent.cancel() }
    }
    @Test fun failedSurfaceChargeReleaseRetriesWithoutReclosingNativeOrGraphics() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source(); val pool = Resources(); var retired=0
        val owner = GiantAndroidCanvas.create(parent,source,pool,binding,1L,{_,_->true},{retired++},{})
        try {
            pool.failSurfaceClose=true
            try { owner.close();fail("failed reservation credited") } catch (_: IllegalStateException) { }
            assertFalse(owner.surfaceReserved);assertTrue(source.closed);assertEquals(GIANT_ANDROID_SURFACE_BYTES,pool.used)
            pool.failSurfaceClose=false;owner.close()
            assertEquals(0uL,pool.used);assertEquals(1,retired);assertEquals(1,source.closeCalls)
        } finally { pool.failSurfaceClose=false;withContext(NonCancellable){owner.close()};parent.cancel() }
    }
    @Test fun failedPipelineRetirementKeepsTheBitmapAndBothChargesQuarantined() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source(); val pool = Resources(); var retireCalls=0
        val owner = GiantAndroidCanvas.create(parent,source,pool,binding,1L,{_,_->true},{
            retireCalls++;error("synthetic pipeline retirement failure")
        },{})
        try {
            withContext(Dispatchers.Main.immediate) { owner.update(Core(),camera,PrecisionFrame()) }
            until { owner.status.ready }
            val bitmap=checkNotNull(owner.frame).tiles.single().image
            repeat(2) {
                try { owner.close();fail("unretired graphics credited") } catch (_:IllegalStateException) { }
                assertFalse(owner.surfaceReserved);assertNull(owner.frame);assertFalse(bitmap.isRecycled)
                assertEquals(GIANT_ANDROID_SURFACE_BYTES+GiantViewportSession.TILE_CHARGE.toULong(),pool.used)
            }
            // The shared session currently records a sticky retirement failure;
            // retry must not counterfeit success or lose this quarantine owner.
            assertEquals(1,retireCalls);assertEquals(0,pool.released);assertTrue(source.closed)
        } finally { parent.cancel() }
    }
    @Test fun parentCancellationNeverReopensTheRetiredSurface() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source();val pool = Resources();var retireCalls=0
        val owner = GiantAndroidCanvas.create(parent,source,pool,binding,1L,{_,_->true},{retireCalls++},{})
        try {
            withContext(Dispatchers.Main.immediate) { owner.update(Core(),camera,PrecisionFrame()) }
            until { owner.status.ready }
            val bitmap=checkNotNull(owner.frame).tiles.single().image
            parent.cancel();until { source.closed }
            assertFalse(owner.surfaceReserved);assertNull(owner.frame);assertFalse(owner.status.ready)
            assertTrue(bitmap.isRecycled);assertEquals(GIANT_ANDROID_SURFACE_BYTES,pool.used)
            owner.close();assertEquals(0uL,pool.used);assertEquals(1,retireCalls)
        } finally { withContext(NonCancellable){owner.close()};parent.cancel() }
    }
    @Test fun actualFrameworkReleaseCallbackPrecedesBitmapAndChargeRelease() = runBlocking {
        val parent = CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
        val source = Source(); val pool = Resources()
        val renderer = AtomicReference<CanvasFrontBufferedRenderer<Int>?>(null)
        val reader = AtomicReference<GiantAndroidCanvas?>(null)
        val painted = CompletableDeferred<Unit>(); val created = CompletableDeferred<Unit>()
        var releaseAck = CompletableDeferred<Unit>().apply { complete(Unit) }
        val drawLock = Any(); var drawAllowed = true
        var surface:SurfaceView? = null; var bitmap:Bitmap? = null; var owner:GiantAndroidCanvas? = null
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            try {
                owner = GiantAndroidCanvas.create(parent,source,pool,binding,1L,{_,_->true},{
                    synchronized(drawLock) { drawAllowed=false }
                    renderer.getAndSet(null)?.let { old ->
                        val joined = CompletableDeferred<Unit>(); releaseAck=joined
                        old.release(true) {
                            try {
                                assertFalse(checkNotNull(reader.get()).surfaceReserved)
                                assertFalse(checkNotNull(bitmap).isRecycled)
                                assertTrue(pool.used>=GIANT_ANDROID_SURFACE_BYTES)
                                joined.complete(Unit)
                            } catch (error:Throwable) { joined.completeExceptionally(error) }
                        }
                    }
                    releaseAck.await()
                },{})
                reader.set(owner)
                scenario.onActivity { activity ->
                    check(checkNotNull(owner).surfaceReserved)
                    val view = SurfaceView(activity); surface=view
                    view.holder.addCallback(object:SurfaceHolder.Callback {
                        override fun surfaceCreated(holder:SurfaceHolder) { created.complete(Unit) }
                        override fun surfaceChanged(holder:SurfaceHolder,format:Int,width:Int,height:Int) = Unit
                        override fun surfaceDestroyed(holder:SurfaceHolder) = Unit
                    })
                    activity.addContentView(view,ViewGroup.LayoutParams(64,64))
                }
                withTimeout(8000) { created.await() }
                withContext(Dispatchers.Main.immediate) {
                    renderer.set(CanvasFrontBufferedRenderer(checkNotNull(surface),object:CanvasFrontBufferedRenderer.Callback<Int> {
                        private fun draw(canvas:Canvas) = synchronized(drawLock) {
                            if(drawAllowed) reader.get()?.matchingFrame(binding,1L,view)?.let { drawGiantAndroidTiles(canvas,it);painted.complete(Unit) }
                            Unit
                        }
                        override fun onDrawFrontBufferedLayer(canvas:Canvas,bufferWidth:Int,bufferHeight:Int,param:Int) { draw(canvas) }
                        override fun onDrawMultiBufferedLayer(canvas:Canvas,bufferWidth:Int,bufferHeight:Int,params:Collection<Int>) { draw(canvas) }
                    }))
                    checkNotNull(owner).update(Core(),camera,PrecisionFrame())
                }
                until { checkNotNull(owner).status.ready }
                bitmap=checkNotNull(checkNotNull(owner).frame).tiles.single().image
                withContext(Dispatchers.Main.immediate) { checkNotNull(renderer.get()).renderFrontBufferedLayer(1) }
                withTimeout(8000) { painted.await() }
                checkNotNull(owner).close()
                assertTrue(releaseAck.isCompleted); assertTrue(checkNotNull(bitmap).isRecycled)
                assertEquals(0uL,pool.used); assertTrue(source.closed)
            } finally {
                withContext(NonCancellable) { owner?.close() }
                scenario.onActivity { (surface?.parent as? ViewGroup)?.removeView(surface) }
                parent.cancel()
            }
        }
    }
    private class Resources:SessionResources {
        @Volatile var used=0uL; var released=0; var refuse=false;var failSurfaceClose=false
        override suspend fun reserveForeign(bytes:ULong):SessionResourceCharge {
            if(refuse)throw GiantImageFailure(GiantImageFailureKind.Memory)
            used+=bytes
            return object:SessionResourceCharge { private var closed=false
                override fun close(){if(!closed){
                    if(failSurfaceClose&&bytes==GIANT_ANDROID_SURFACE_BYTES)error("synthetic charge failure")
                    closed=true;used-=bytes;released++}} }
        }
    }
    private class Source:GiantImage {
        @Volatile var closed=false
        @Volatile var failClose=false
        var closeCalls=0
        override suspend fun viewport(view:GiantViewport)=GiantViewportPlan("a".repeat(64),"b".repeat(64),0u,listOf(GiantTileAddress(0u,0u,0u)),emptyList())
        override suspend fun tile(address:GiantTileAddress,foreign:SessionResourceCharge)=Companion.tile()
        override suspend fun close(){closeCalls++;if(failClose)error("synthetic close failure");closed=true}
    }
    private class Core:WorkbenchCore {
        override fun newId(unixMs:ULong):String=error("unused")
        override fun newDeviceId():String=error("unused")
        override suspend fun create(options:CreateProject):WorkbenchProject=error("unused")
        override suspend fun open(path:String):WorkbenchProject=error("unused")
        override suspend fun layoutText(text:String,font:String,size:Float):TextLayout=error("unused")
        override fun cameraMatrix(camera:Camera,inverse:Boolean)=Transform(camera.scale,0.0,0.0,camera.scale)
        override fun mapPoints(camera:Camera,inverse:Boolean,points:List<Point>)=points.map { Point(it.x+camera.center.x,it.y+camera.center.y) }
    }
    private suspend fun until(check:()->Boolean){withTimeout(3000){while(!check())delay(5)}}
    private companion object {
        val binding=WorkflowBinding("project","document",1uL,"c".repeat(64))
        val camera=Camera(Point(0.0,0.0),1.0,0.0,256.0,256.0)
        val view=GiantViewport(0.0,0.0,256.0,256.0,1.0,0.0,0.0,1.0)
        fun tile(width:Int=256,height:Int=256,rgba:ByteArray=ByteArray(width*height*4){if(it%4==3)(-1).toByte() else 0}) =
            GiantTilePixels("a".repeat(64),"b".repeat(64),GiantTileAddress(0u,0u,0u),0u,0u,width.toUInt(),height.toUInt(),width.toUInt(),height.toUInt(),rgba)
    }
}
