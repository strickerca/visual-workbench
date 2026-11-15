package com.visualworkbench.android.editor

import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class StrokeTurnsTest {
    @Test fun strokes_save_in_pen_down_order_even_when_a_later_one_is_ready_first(): Unit = runBlocking {
        val turns = StrokeTurns()
        val saved = mutableListOf<Int>()
        val ready = List(3) { CompletableDeferred<Unit>() }
        val strokes = (0..2).map { number ->
            val turn = turns.enter() // taken at pen-down, in drawing order
            launch(start = CoroutineStart.UNDISPATCHED) {
                try { ready[number].await(); turn.awaitEarlier(); saved += number } finally { turn.done() }
            }
        }
        // Ink finishes in the opposite order.
        ready[2].complete(Unit); yield(); assertTrue(saved.isEmpty())
        ready[1].complete(Unit); yield(); assertTrue(saved.isEmpty())
        ready[0].complete(Unit); strokes.joinAll()
        assertEquals(listOf(0, 1, 2), saved)
    }
    @Test fun the_first_stroke_waits_for_nobody_and_a_finished_turn_never_blocks_a_later_one(): Unit = runBlocking {
        val turns = StrokeTurns()
        val first = turns.enter()
        withTimeout(1_000) { first.awaitEarlier() }
        first.done(); first.done()
        val second = turns.enter()
        withTimeout(1_000) { second.awaitEarlier() }
        second.done()
    }
    @Test fun a_cancelled_or_failed_stroke_still_lets_the_next_one_save(): Unit = runBlocking {
        val turns = StrokeTurns()
        val failing = turns.enter(); val waiting = turns.enter(); val next = turns.enter()
        // The middle stroke is cancelled while it waits for the first.
        val cancelled = launch(start = CoroutineStart.UNDISPATCHED) { try { waiting.awaitEarlier(); fail("not reached") } finally { waiting.done() } }
        cancelled.cancelAndJoin()
        var saved = false
        val last = launch(start = CoroutineStart.UNDISPATCHED) { try { next.awaitEarlier(); saved = true } finally { next.done() } }
        last.join(); assertTrue(saved)
        // The first one finishing afterwards changes nothing for those already through.
        failing.done()
    }
}
