package com.visualworkbench.android.remote

import android.os.Handler
import android.view.Surface
import org.junit.Assert.assertEquals
import org.junit.Test

class RemoteDecoderWaitsTest {
    private val states = listOf(false to false, false to true, true to false, true to true)
    /** One pass of the decoder worker: (wait on the decoder in microseconds, wait for the callback in milliseconds). */
    private fun remoteDecoderWaits(quick: Boolean, inputQueued: Boolean, outputReleased: Boolean): Pair<Long, Long> {
        val output = remoteDecoderOutputWaitMicros(quick, inputQueued, outputReleased)
        return Pair(output, remoteDecoderCallbackWaitMillis(output))
    }

    @Test fun switched_off_never_waits_on_the_decoder_and_always_waits_four_milliseconds_for_the_callback() {
        // Exactly the two waits the worker has always used, in every state.
        for ((inputQueued, outputReleased) in states)
            assertEquals(Pair(0L, 4L), remoteDecoderWaits(false, inputQueued, outputReleased))
    }

    @Test fun switched_on_waits_on_the_decoder_only_while_a_frame_is_being_decoded() {
        // A frame is in the decoder and not yet shown: wait there, not on the callback.
        assertEquals(Pair(4_000L, 0L), remoteDecoderWaits(true, inputQueued = true, outputReleased = false))
        // Otherwise nothing changes: no input buffer yet, or the frame is already
        // on its way to the screen and only its callback is awaited.
        assertEquals(Pair(0L, 4L), remoteDecoderWaits(true, inputQueued = false, outputReleased = false))
        assertEquals(Pair(0L, 4L), remoteDecoderWaits(true, inputQueued = true, outputReleased = true))
        assertEquals(Pair(0L, 4L), remoteDecoderWaits(true, inputQueued = false, outputReleased = true))
        // One pass never waits longer than the four milliseconds it always could.
        for (quick in listOf(false, true)) for ((inputQueued, outputReleased) in states) {
            val (outputMicros, callbackMillis) = remoteDecoderWaits(quick, inputQueued, outputReleased)
            assertEquals(4_000L, outputMicros + callbackMillis * 1_000L)
        }
    }

    @Test fun an_engine_that_cannot_wait_answers_at_once_with_its_ordinary_output() {
        var asked = 0
        val engine = object : DecoderEngine {
            override fun start(config: DecoderConfig, surface: Surface, callbacks: Handler,
                onRendered: (Long, Long, Long) -> Unit): DecoderAdmission = error("unused")
            override fun queue(bytes: ByteArray, ptsUs: Long): Boolean = false
            override fun output(): DecoderOutput { asked++; return DecoderOutput.Buffer(3, 77L, 0) }
            override fun outputDimensions(): Pair<Int, Int> = Pair(2, 2)
            override fun discardOutput(index: Int) = Unit
            override fun renderOutput(index: Int, requestedLocalNanos: Long) = Unit
            override fun stop() = Unit
            override fun release() = Unit
        }
        assertEquals(DecoderOutput.Buffer(3, 77L, 0), engine.awaitOutput(4_000L))
        assertEquals(1, asked)
    }
}
