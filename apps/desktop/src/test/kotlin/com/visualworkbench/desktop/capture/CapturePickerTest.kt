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
    @Test fun failedModalCloseRetainsTheExactLeaseForRetry() {
        var attempts = 0
        var acquisitions = 0
        val modal = PickerModal {
            acquisitions++
            AutoCloseable { if (++attempts == 1) error("pending") }
        }
        modal.set(true)
        assertTrue(runCatching { modal.set(false) }.isFailure)
        modal.set(true)
        modal.set(false)
        assertEquals(1, acquisitions)
        assertEquals(2, attempts)
    }
    @Test fun operationGenerationRejectsCancelledAndRetiredResults() = runBlocking {
        val owner = PickerOperationOwner(this) {}
        val entered = CompletableDeferred<Long>()
        val gate = CompletableDeferred<Unit>()
        assertTrue(owner.start { ticket -> entered.complete(ticket); gate.await() })
        val ticket = entered.await()
        assertTrue(owner.current(ticket))
        assertFalse(owner.start {})
        owner.invalidate()
        assertFalse(owner.current(ticket))
        owner.retire { gate.complete(Unit) }
        assertTrue(owner.accepting)
        owner.retire(permanent = true) {}
        assertFalse(owner.accepting)
        assertFalse(owner.start {})
    }
    @Test fun dismissalSealsAdmissionBeforeJoinAndIgnoresCallerCancellation() = runBlocking {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val owner = PickerOperationOwner(scope) {}
        val entered = CompletableDeferred<Unit>()
        val gate = CompletableDeferred<Unit>()
        val cleaned = AtomicInteger()
        owner.start { withContext(NonCancellable) { entered.complete(Unit); gate.await() } }
        entered.await()
        val close = launch(start = CoroutineStart.UNDISPATCHED) { owner.retire { cleaned.incrementAndGet() } }
        assertFalse(owner.accepting)
        assertFalse(owner.start {})
        close.cancel()
        assertFalse(close.isCompleted)
        gate.complete(Unit)
        withTimeout(2000) { close.join() }
        assertEquals(1, cleaned.get())
        assertTrue(owner.accepting)
        scope.cancel()
    }
    @Test fun concurrentDismissalsDoNotReopenAdmissionBetweenCleanups() = runBlocking {
        val owner = PickerOperationOwner(this) {}
        val firstGate = CompletableDeferred<Unit>()
        val secondGate = CompletableDeferred<Unit>()
        val first = launch(start = CoroutineStart.UNDISPATCHED) { owner.retire { firstGate.await() } }
        val second = launch(start = CoroutineStart.UNDISPATCHED) { owner.retire { secondGate.await() } }
        firstGate.complete(Unit)
        first.join()
        assertFalse(owner.accepting)
        secondGate.complete(Unit)
        second.join()
        assertTrue(owner.accepting)
    }
    @Test fun cancelledScopeCannotAdmitWorkOrStrandBusyState() = runBlocking {
        val scope = CoroutineScope(Job())
        val states = mutableListOf<Boolean>()
        val owner = PickerOperationOwner(scope, states::add)
        scope.cancel()
        assertFalse(owner.start { error("must not run") })
        assertTrue(states.isEmpty())
    }
    @Test fun cleanupFailureCanBeRetriedAfterPermanentAdmissionSeal() = runBlocking {
        val owner = PickerOperationOwner(this) {}
        try { owner.retire(permanent = true) { error("pending") }; fail("must refuse") }
        catch (_: IllegalStateException) {}
        assertFalse(owner.accepting)
        var retired = false
        owner.retire(permanent = true) { retired = true }
        assertTrue(retired)
    }
    @Test fun malformedPreviewExtentsAndPayloadAreRefusedBeforeBitmapAllocation() {
        val valid = PickerPreview("a", 2u, 1u, 2u, 1u, ByteArray(8))
        validatePickerPreview(valid)
        for (invalid in listOf(valid.copy(sourceAssetId = ""), valid.copy(sourceWidth = 0u),
            valid.copy(sourceHeight = 32769u), valid.copy(width = 0u), valid.copy(width = 3u, rgba = ByteArray(12)),
            valid.copy(height = 1025u), valid.copy(rgba = ByteArray(9)))) {
            assertTrue(runCatching { validatePickerPreview(invalid) }.isFailure)
        }
    }
    @Test fun nonFiniteAndRotatedDisplayCamerasAreRefused() {
        val preview = PickerPreview("a", 2u, 1u, 2u, 1u, ByteArray(8))
        val camera = Camera(Point(1.0, .5), 1.0, 0.0, 2.0, 1.0)
        for (invalid in listOf(camera.copy(scale = Double.NaN), camera.copy(scale = 0.0),
            camera.copy(rotation = .1), camera.copy(viewportWidth = Double.POSITIVE_INFINITY),
            camera.copy(center = Point(Double.NaN, 0.0)))) {
            assertTrue(runCatching { pickerDisplayTransform(preview, invalid) }.isFailure)
        }
    }
    @Test fun retentionChecksOwnershipBeforeWritingRecoveryOrDeletingEmptyWorkspace() {
        val projects = Files.createTempDirectory("vw-picker-test").toRealPath()
        val work = PickerWorkspace.create(projects)
        val marker = work.path.resolve("owner-v1")
        val token = Files.readAllBytes(marker)
        try {
            Files.writeString(marker, "foreign")
            assertTrue(runCatching { work.retain() }.isFailure)
            assertTrue(Files.exists(work.path))
            Files.write(work.path.resolve("capture.png"), byteArrayOf(7))
            assertTrue(runCatching { work.retain() }.isFailure)
            assertFalse(Files.exists(work.path.resolve("recovery.txt")))
        } finally {
            Files.write(marker, token); work.release()
            Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects)
        }
    }
    @Test fun symlinkedOriginalIsNeverFollowedRetainedOrDeleted() {
        val projects = Files.createTempDirectory("vw-picker-test").toRealPath()
        val foreign = Files.createTempFile("vw-picker-foreign", ".bin")
        val work = PickerWorkspace.create(projects)
        val link = work.path.resolve("capture.png")
        try {
            Files.write(foreign, byteArrayOf(9))
            try { Files.createSymbolicLink(link, foreign) }
            catch (unsupported: UnsupportedOperationException) { org.junit.Assume.assumeNoException(unsupported) }
            catch (unavailable: java.nio.file.FileSystemException) { org.junit.Assume.assumeNoException(unavailable) }
            assertTrue(runCatching { work.retain() }.isFailure)
            assertTrue(runCatching { work.release() }.isFailure)
            assertArrayEquals(byteArrayOf(9), Files.readAllBytes(foreign))
            assertTrue(Files.isSymbolicLink(link))
        } finally {
            Files.deleteIfExists(link); work.release(); Files.delete(foreign)
            Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects)
        }
    }
    @Test fun successfulWorkspaceRetirementIsIdempotent() {
        val projects = Files.createTempDirectory("vw-picker-test").toRealPath()
        try {
            val work = PickerWorkspace.create(projects)
            work.release(); work.release(); work.retain()
            assertFalse(Files.exists(work.path))
        } finally { Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects) }
    }

    @Test fun cancellationAfterExclusionSettlementStillCannotPublishReady() = runBlocking {
        val entered = CompletableDeferred<Unit>()
        val gate = CompletableDeferred<Unit>()
        var outcome: PickerProtection? = null
        val call = launch(start = CoroutineStart.UNDISPATCHED) {
            outcome = initializePickerProtection {
                withContext(NonCancellable) { entered.complete(Unit); gate.await() }
            }
        }
        entered.await(); call.cancel(); gate.complete(Unit)
        withTimeout(2000) { call.join() }
        assertNull(outcome)
    }

    @Test fun encodedPreviewBufferRejectsGrowthAndInvalidRanges() {
        val output = PickerPngBuffer(4)
        output.write(byteArrayOf(1, 2, 3), 0, 3)
        assertTrue(runCatching { output.write(byteArrayOf(4, 5), 0, 2) }.isFailure)
        assertTrue(runCatching { output.write(byteArrayOf(4), -1, 1) }.isFailure)
        output.write(4)
        assertTrue(runCatching { output.write(5) }.isFailure)
        assertArrayEquals(byteArrayOf(1, 2, 3, 4), output.bytes())
    }

    @Test fun unknownNativeRetirementNeverCreditsASecondNoOpDestruction() {
        var calls = 0
        val failure = IllegalStateException("unknown retirement")
        val release = PickerReleaseAction { calls++; throw failure }
        repeat(2) { assertSame(failure, runCatching { release.close() }.exceptionOrNull()) }
        assertEquals(1, calls)
        var successes = 0
        val complete = PickerReleaseAction { successes++ }
        complete.close(); complete.close()
        assertEquals(1, successes)
    }

    @Test fun saveAndRegionRequireTheExactDisplayedSnapshot() {
        val preview = PickerPreview("a", 2u, 1u, 2u, 1u, ByteArray(8))
        val pending = CapturePickerState(open = true, preview = preview)
        assertFalse(pending.acceptsPreview(preview))
        val displayed = pending.copy(previewReady = true)
        assertTrue(displayed.acceptsPreview(preview))
        assertFalse(displayed.acceptsPreview(preview.copy()))
        assertFalse(displayed.acceptsPreview(null))
        for (retired in listOf(displayed.copy(open = false), displayed.copy(stopping = true),
            displayed.copy(cleanupPending = true), displayed.copy(preview = null))) {
            assertFalse(retired.acceptsPreview(preview))
        }
    }

    @Test fun finalDirectoryDeleteFailureCanRetryAfterTheMarkerWasRemoved() {
        val projects = Files.createTempDirectory("vw-picker-test").toRealPath()
        var attempts = 0
        val work = PickerWorkspace.create(projects) {
            if (++attempts == 1) throw java.io.IOException("busy directory")
            Files.delete(it)
        }
        try {
            Files.write(work.path.resolve("capture.png"), byteArrayOf(7))
            assertTrue(runCatching { work.release() }.isFailure)
            assertTrue(Files.isDirectory(work.path)); assertFalse(Files.exists(work.path.resolve("owner-v1")))
            val keyed = Files.readAttributes(work.path, java.nio.file.attribute.BasicFileAttributes::class.java).fileKey() != null
            if (keyed) { work.release(); assertEquals(2, attempts) }
            else {
                assertTrue(runCatching { work.release() }.isFailure); assertEquals(1, attempts)
                Files.delete(work.path) // Exact empty test fixture; the production owner refused it.
                work.release()
            }
            work.release(); assertFalse(Files.exists(work.path))
        } finally {
            work.release(); Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects)
        }
    }

    @Test fun finalDeletionRetryRefusesAnUnmarkedReplacementDirectory() {
        val projects = Files.createTempDirectory("vw-picker-test").toRealPath()
        var attempts = 0
        val work = PickerWorkspace.create(projects) {
            if (++attempts == 1) throw java.io.IOException("busy directory")
            Files.delete(it)
        }
        val held = projects.resolve("held-original")
        try {
            assertTrue(runCatching { work.release() }.isFailure)
            Files.move(work.path, held); Files.createDirectory(work.path)
            assertTrue(runCatching { work.release() }.isFailure)
            assertTrue(Files.isDirectory(work.path)); assertEquals(1, attempts)
            Files.delete(work.path); Files.move(held, work.path)
            val keyed = Files.readAttributes(work.path, java.nio.file.attribute.BasicFileAttributes::class.java).fileKey() != null
            if (keyed) work.release()
            else { assertTrue(runCatching { work.release() }.isFailure); Files.delete(work.path); work.release() }
        } finally {
            if (Files.exists(held)) { Files.deleteIfExists(work.path); Files.move(held, work.path) }
            work.release(); Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects)
        }
    }

    @Test fun finalDeletionRetryCannotDeleteANewChild() {
        val projects = Files.createTempDirectory("vw-picker-test").toRealPath()
        var attempts = 0
        val work = PickerWorkspace.create(projects) {
            if (++attempts == 1) throw java.io.IOException("busy directory")
            Files.delete(it)
        }
        val foreign = work.path.resolve("foreign")
        try {
            assertTrue(runCatching { work.release() }.isFailure)
            Files.writeString(foreign, "keep")
            assertTrue(runCatching { work.retain() }.isFailure)
            assertEquals("keep", Files.readString(foreign)); assertEquals(1, attempts)
            Files.delete(foreign)
            val keyed = Files.readAttributes(work.path, java.nio.file.attribute.BasicFileAttributes::class.java).fileKey() != null
            if (keyed) work.retain()
            else { assertTrue(runCatching { work.retain() }.isFailure); Files.delete(work.path); work.retain() }
            assertFalse(Files.exists(work.path))
        } finally {
            Files.deleteIfExists(foreign); work.release()
            Files.delete(projects.resolve(".picker-inbox-v1")); Files.delete(projects)
        }
    }

}
