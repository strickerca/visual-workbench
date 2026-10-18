package com.visualworkbench.desktop.capture

import com.visualworkbench.bindings.host.PickerPreview
import com.visualworkbench.shared.Camera
import com.visualworkbench.shared.Point
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicInteger

class CapturePickerTest {
    @Test fun refusedCaptureExclusionShowsEditorAndDisablesCapture() = runBlocking {
        assertFalse(PickerProtection.Pending.editorVisible)
        val refused=initializePickerProtection{throw IllegalStateException("unsupported")}
        assertTrue(refused.editorVisible); assertFalse(refused.captureAllowed)
    }
    @Test fun exclusionSuccessAloneEnablesPicker() = runBlocking {
        val ready=initializePickerProtection{}
        assertTrue(ready.editorVisible); assertTrue(ready.captureAllowed)
    }
    @Test fun cancelledStartupCannotPublishAnExclusionOutcome() = runBlocking {
        var result:PickerProtection?=null
        try { result=initializePickerProtection{throw CancellationException("closed")} }
        catch (_:CancellationException) { }
        assertNull(result)
    }
    @Test fun fractionalLetterboxUsesTheSameOriginalPixelTransformAsRegionMapping() {
        for((w,h) in listOf(601 to 337,377 to 613,999 to 511)) {
            val source=PickerPreview("source",3840u,2160u,1024u,576u,byteArrayOf())
            val camera=Camera(Point(1920.0,1080.0),minOf(w/3840.0,h/2160.0),0.0,w.toDouble(),h.toDouble())
            val display=pickerDisplayTransform(source,camera)
            for(point in listOf(Point(0.0,0.0),Point(10.25,21.75),Point(2301.25,1833.5),Point(3840.0,2160.0))) {
                // These are the exact scale/translate fields used by Canvas,
                // not a second independently rounded destination rectangle.
                val x=display.left+point.x/3840.0*1024*display.scaleX
                val y=display.top+point.y/2160.0*576*display.scaleY
                val inverseX=(x-camera.viewportWidth/2)/camera.scale+camera.center.x
                val inverseY=(y-camera.viewportHeight/2)/camera.scale+camera.center.y
                assertEquals(point.x,inverseX,.001); assertEquals(point.y,inverseY,.001)
            }
        }
    }
    @Test fun cancellationJoinsNativeProducerAndReleasesLateOwnerExactlyOnce() = runBlocking {
        val producers = CoroutineScope(SupervisorJob()+Dispatchers.Default)
        val entered=CompletableDeferred<Unit>(); val gate=CompletableDeferred<Unit>(); val released=AtomicInteger(); val signals=AtomicInteger()
        val call=launch { settledPicker(producers, { _:Any -> released.incrementAndGet() }, { signals.incrementAndGet() }) {
            entered.complete(Unit); gate.await(); Any()
        } }
        try { withTimeout(2000){entered.await()}; call.cancel(); yield(); assertFalse(call.isCompleted)
            gate.complete(Unit); withTimeout(2000){call.join()}; assertEquals(1,released.get()); assertEquals(1,signals.get())
        } finally { gate.complete(Unit); withContext(NonCancellable){call.cancelAndJoin()}; producers.cancel() }
    }
    @Test fun throwingCancellationSignalStillJoinsAndReleasesProducer() = runBlocking {
        val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default); val entered=CompletableDeferred<Unit>(); val gate=CompletableDeferred<Unit>(); val released=AtomicInteger()
        val call=launch { settledPicker(producers, { _:Any -> released.incrementAndGet() }, { error("signal") }) { entered.complete(Unit); gate.await(); Any() } }
        try { withTimeout(2000){entered.await()}; call.cancel(); gate.complete(Unit); withTimeout(2000){call.join()}; assertEquals(1,released.get()) }
        finally { gate.complete(Unit); withContext(NonCancellable){call.cancelAndJoin()}; producers.cancel() }
    }
    @Test fun alreadyCancelledCallerCannotCreateAWorkspaceOrNativeOwner() = runBlocking {
        val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default); val calls=AtomicInteger()
        val call=launch(start=CoroutineStart.LAZY) { settledPicker(producers) { calls.incrementAndGet() } }
        call.cancel(); call.start(); call.join(); assertEquals(0,calls.get()); producers.cancel()
    }
    @Test fun deliveredOwnerBelongsToCallerAndIsNotDisposedByHandoff() = runBlocking {
        val producers=CoroutineScope(SupervisorJob()+Dispatchers.Default); val closed=AtomicInteger(); val value=Any()
        try { assertSame(value, settledPicker(producers, { _:Any -> closed.incrementAndGet() }) { value }); assertEquals(0,closed.get()) }
        finally { producers.cancel() }
    }
    @Test fun pickerModalOwnsOneLeaseAndNeverReleasesAnotherDialog() {
        var leases=1; var acquisitions=0
        val modal=PickerModal { acquisitions++; leases++; AutoCloseable { leases-- } }
        modal.set(true); modal.set(true); assertEquals(2,leases); modal.set(false); modal.set(false)
        assertEquals(1,leases); assertEquals(1,acquisitions)
    }
    @Test fun rgbaDisplayConversionPreservesChannelOrderAndAlpha() {
        val source=PickerPreview("a",2u,1u,2u,1u,byteArrayOf(255.toByte(),2,3,128.toByte(),4,5,255.toByte(),255.toByte()))
        assertArrayEquals(intArrayOf(0x80ff0203.toInt(),0xff0405ff.toInt()),pickerArgb(source))
        assertTrue(runCatching{pickerArgb(source.copy(width=1025u))}.isFailure)
        assertTrue(runCatching{pickerArgb(source.copy(rgba=ByteArray(7)))}.isFailure)
    }
    @Test fun emptyPrivateWorkspaceIsRemovedButCompletedOriginalIsRetained() {
        val projects=Files.createTempDirectory("vw-picker-test").toRealPath()
        try { val empty=PickerWorkspace.create(projects); empty.retain(); assertFalse(Files.exists(empty.path))
            val complete=PickerWorkspace.create(projects); Files.write(complete.path.resolve("capture.png"),byteArrayOf(1,2,3)); complete.retain()
            assertArrayEquals(byteArrayOf(1,2,3),Files.readAllBytes(complete.path.resolve("capture.png")))
            assertTrue(Files.exists(complete.path.resolve("recovery.txt"))); complete.release()
        } finally { Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects) }
    }
    @Test fun foreignMarkerAndUnexpectedChildRefuseCleanupWithoutDeletingOriginal() {
        val projects=Files.createTempDirectory("vw-picker-test").toRealPath(); val work=PickerWorkspace.create(projects)
        val marker=work.path.resolve("owner-v1"); val token=Files.readAllBytes(marker); val original=work.path.resolve("capture.png")
        try { Files.write(original,byteArrayOf(7)); Files.writeString(marker,"foreign")
            assertTrue(runCatching{work.release()}.isFailure); assertTrue(Files.exists(original)); Files.write(marker,token)
            val unrelated=work.path.resolve("unrelated"); Files.writeString(unrelated,"keep")
            assertTrue(runCatching{work.release()}.isFailure); assertTrue(Files.exists(original)); Files.delete(unrelated); work.release()
        } finally { if(Files.exists(marker)){Files.write(marker,token); work.release()}; Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects) }
    }
    @Test fun retainedCaptureCountCannotGrowWithoutBoundOrAdoptOldOwners() {
        val projects=Files.createTempDirectory("vw-picker-test").toRealPath(); val owners=mutableListOf<PickerWorkspace>()
        try { repeat(8){owners+=PickerWorkspace.create(projects)}
            assertTrue(runCatching{PickerWorkspace.create(projects)}.isFailure)
            assertEquals(8,Files.list(projects.resolve(".picker-inbox-v1")).use{it.count()}.toInt())
        } finally { owners.forEach{it.release()}; Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects) }
    }
}
