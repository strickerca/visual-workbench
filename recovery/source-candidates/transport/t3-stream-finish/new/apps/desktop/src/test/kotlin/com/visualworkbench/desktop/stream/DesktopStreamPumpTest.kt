package com.visualworkbench.desktop.stream

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

class DesktopStreamPumpTest {
    @Test fun cancelledPublicationRetainsPacketAndProducerUntilNativeSettlement(): Unit = runBlocking {
        val trace = mutableListOf<String>(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val packet = Packet(HostPacketInfo.Tiles(7uL, 3u), byteArrayOf(1, 2, 3), trace)
        val producer = Producer(trace).apply { polling = { _, _, _ -> packet } }
        val publisher = Publisher(trace).apply { publishing = { bytes ->
            assertFalse(packet.closed); assertArrayEquals(byteArrayOf(1, 2, 3), bytes)
            entered.complete(Unit); withContext(NonCancellable) { release.await() }; assertFalse(packet.closed); true
        } }
        val pump = DesktopStreamPump(this)
        assertTrue(pump.start({ producer }, { publisher }))
        try {
            withTimeout(5000) { entered.await() }
            val stop = launch { pump.stop() }; yield()
            assertFalse(packet.closed); assertTrue(pump.state.value.running)
            assertFalse(pump.start({ error("new producer") }, { publisher }))
            release.complete(Unit); withTimeout(5000) { stop.join() }
            assertTrue(packet.closed)
            assertTrue(trace.indexOf("packet-close") < trace.indexOf("producer-close"))
            assertTrue(trace.indexOf("producer-close") < trace.indexOf("publisher-close"))
            assertFalse(pump.state.value.running)
        } finally { release.complete(Unit); pump.close() }
    }
    @Test fun rejectedMediaFeedbackIsConsumedExactlyOnceAndRequestsRepair(): Unit = runBlocking {
        val trace = mutableListOf<String>(); val second = CompletableDeferred<Triple<ULong, Boolean, Boolean>>()
        var calls = 0
        val producer = Producer(trace).apply { polling = { offer, accepted, repair ->
            calls++
            if (calls == 1) { assertEquals(0uL, offer); assertFalse(accepted); assertTrue(repair); Packet(HostPacketInfo.Tiles(91uL, 1u), byteArrayOf(4), trace) }
            else { if (calls == 2) second.complete(Triple(offer, accepted, repair)); Packet(HostPacketInfo.Idle, byteArrayOf(), trace) }
        } }
        val publisher = Publisher(trace).apply { publishing = { false } }
        val pump = DesktopStreamPump(this)
        try {
            assertTrue(pump.start({ producer }, { publisher }))
            assertEquals(Triple(91uL, false, false), withTimeout(5000) { second.await() })
            assertEquals(0uL, pump.state.value.publishedOffers)
        } finally { pump.close() }
    }
    @Test fun exactOriginalRequestIsAnsweredOnceWhilePaused(): Unit = runBlocking {
        val trace = mutableListOf<String>(); val supplied = CompletableDeferred<Unit>(); var originals = 0
        val frame = CaptureStreamFrame("session", 9uL, 3uL, 2u, 1u, -10, 20, 1.5, 5u, 99, 50, "windows")
        val identity = CaptureImportDescriptor(true, "session", 9uL, 5u, "windows", "window", -10, 20, 2u, 1u, 1.5, 99uL, 50, 123uL, "", "a".repeat(64))
        val producer = Producer(trace).apply { originalCall = { request ->
            assertEquals(frame, request.frame); originals++; Packet(HostPacketInfo.Original(identity, 2u), byteArrayOf(8, 9), trace)
        } }
        val publisher = Publisher(trace).apply {
            current = current.copy(paused = true, original = CaptureOriginalRequest(6uL, "draft", frame))
            originalCall = { sequence, actual, data -> assertEquals(6uL, sequence); assertEquals(identity, actual); assertArrayEquals(byteArrayOf(8, 9), data); supplied.complete(Unit) }
        }
        val pump = DesktopStreamPump(this)
        try {
            assertTrue(pump.start({ producer }, { publisher }))
            withTimeout(5000) { supplied.await(); producer.beat.await() }
            assertEquals(1, originals); assertTrue(pump.state.value.paused)
        } finally { pump.close() }
    }
    @Test fun lateOpenedProducerClosesWithoutPublishingAfterCancellation(): Unit = runBlocking {
        val trace = mutableListOf<String>(); val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val producer = Producer(trace); var openedPublisher = false
        val pump = DesktopStreamPump(this)
        try {
            assertTrue(pump.start({ withContext(NonCancellable) { entered.complete(Unit); release.await() }; producer }, { openedPublisher = true; Publisher(trace) }))
            withTimeout(5000) { entered.await() }; val stop = launch { pump.stop() }; yield()
            assertFalse(stop.isCompleted); assertFalse(pump.start({ producer }, { error("not reached") }))
            release.complete(Unit); withTimeout(5000) { stop.join() }
            assertFalse(openedPublisher); assertTrue(trace.contains("producer-close"))
        } finally { release.complete(Unit); pump.close() }
    }
    @Test fun failedCleanupCannotReadmitAnotherProducer(): Unit = runBlocking {
        val trace = mutableListOf<String>(); val producer = Producer(trace).apply { failClose = true }
        val pump = DesktopStreamPump(this)
        try {
            assertTrue(pump.start({ producer }, { Publisher(trace) }))
            withTimeout(5000) { producer.polled.await() }; pump.stop()
            assertNotNull(pump.state.value.failure)
            assertFalse(pump.start({ error("not reached") }, { error("not reached") }))
        } finally { pump.close() }
    }
    @Test fun stopBeforeDispatchDoesNotOpenCapture(): Unit = runBlocking {
        val pump = DesktopStreamPump(this); var entered = false
        assertTrue(pump.start({ entered = true; error("not reached") }, { error("not reached") }))
        pump.stop(); assertFalse(entered); assertFalse(pump.state.value.running); pump.close()
    }

    private class Packet(override val info: HostPacketInfo, private val bytes: ByteArray, private val trace: MutableList<String>) : HostPacket {
        var closed = false; private var taken = false
        override suspend fun payload(): ByteArray { check(!taken && !closed); taken = true; return bytes }
        override suspend fun close() { check(!closed); closed = true; trace += "packet-close" }
    }
    private class Producer(private val trace: MutableList<String>) : HostProducer {
        val polled = CompletableDeferred<Unit>(); val beat = CompletableDeferred<Unit>()
        var failClose = false
        var polling: suspend (ULong, Boolean, Boolean) -> HostPacket = { _, _, _ -> Packet(HostPacketInfo.Idle, byteArrayOf(), trace) }
        var originalCall: suspend (CaptureOriginalRequest) -> HostPacket = { Packet(HostPacketInfo.OriginalUnavailable, byteArrayOf(), trace) }
        override suspend fun poll(offer: ULong, admitted: Boolean, repair: Boolean): HostPacket { polled.complete(Unit); return polling(offer, admitted, repair) }
        override suspend fun original(request: CaptureOriginalRequest): HostPacket = originalCall(request)
        override suspend fun heartbeat(): HostPacket { beat.complete(Unit); return Packet(HostPacketInfo.Idle, byteArrayOf(), trace) }
        override fun cancel() { trace += "producer-cancel" }
        override suspend fun close() { trace += "producer-close"; if (failClose) throw IllegalStateException("synthetic") }
    }
    private class Publisher(private val trace: MutableList<String>) : WorkbenchCapturePublisher {
        var current = CapturePublicationSignal(1uL, 1uL, 3uL, false, false, 1uL, null, null)
        var publishing: suspend (ByteArray) -> Boolean = { true }
        var originalCall: suspend (ULong, CaptureImportDescriptor, ByteArray) -> Unit = { _, _, _ -> }
        override val signals: Flow<CapturePublicationSignal> = emptyFlow()
        override fun signal(): CapturePublicationSignal = current
        override suspend fun publishFrame(encoded: ByteArray): Boolean = publishing(encoded)
        override suspend fun provideOriginal(requestSequence: ULong, identity: CaptureImportDescriptor, encoded: ByteArray) { originalCall(requestSequence, identity, encoded) }
        override suspend fun originalUnavailable(requestSequence: ULong) = Unit
        override suspend fun close() { trace += "publisher-close" }
    }
}