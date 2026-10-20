package com.visualworkbench.android.stream

import android.view.MotionEvent
import com.visualworkbench.android.input.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOf

internal fun runStreamBoundaryRepairContract(): Int = runBlocking {
    withTimeout(10_000) {
        var cases = 0
        val caps = StylusCapabilities(true, false, false, true, false, false)
        val down = PenSample(1, PointerTool.Pen, 2.0, 3.0, 10, 1f, 1f, null, null, 0)
        fun packet(action: Int, time: Long, start: Long) = PointerPacket(action, 1,
            action == MotionEvent.ACTION_CANCEL, start, listOf(PointerFrame(time, listOf(down.copy(timeMs = time)))), caps)
        run {
            var freezes = 0
            val fence = StreamInputFence({ _, _ -> true }, { freezes++ })
            val router = CanvasGestureRouter(fence, 8.0); val sink = BoundarySink()
            router.accept(packet(MotionEvent.ACTION_DOWN, 10, 10)); router.accept(packet(MotionEvent.ACTION_MOVE, 11, 10))
            router.accept(packet(MotionEvent.ACTION_UP, 12, 10))
            router.accept(packet(MotionEvent.ACTION_DOWN, 20, 20)); router.accept(packet(MotionEvent.ACTION_MOVE, 21, 20))
            fence.attach(sink); router.accept(packet(MotionEvent.ACTION_UP, 22, 20))
            check(freezes == 1 && sink.downs.map { it.timeMs } == listOf(10L))
            check(sink.finished.map { it.timeMs } == listOf(11L, 12L) && sink.extended.isEmpty())
            router.accept(packet(MotionEvent.ACTION_DOWN, 30, 30)); router.accept(packet(MotionEvent.ACTION_UP, 31, 30))
            check(sink.downs.map { it.timeMs } == listOf(10L, 30L) && sink.finished.last().timeMs == 31L)
            cases++
        }
        run {
            val fence = StreamInputFence({ _, _ -> true }, {})
            val router = CanvasGestureRouter(fence, 8.0); val sink = BoundarySink()
            router.accept(packet(MotionEvent.ACTION_DOWN, 10, 10)); router.accept(packet(MotionEvent.ACTION_UP, 11, 10))
            router.accept(packet(MotionEvent.ACTION_DOWN, 20, 20)); router.accept(packet(MotionEvent.ACTION_CANCEL, 21, 20))
            fence.attach(sink)
            check(sink.downs.single().timeMs == 10L && sink.finished.single().timeMs == 11L); cases++
        }
        run {
            var freezes = 0
            val fence = StreamInputFence({ _, _ -> true }, { freezes++ })
            val router = CanvasGestureRouter(fence, 8.0); val sink = BoundarySink()
            router.accept(packet(MotionEvent.ACTION_DOWN, 10, 10)); router.accept(packet(MotionEvent.ACTION_CANCEL, 11, 10))
            router.accept(packet(MotionEvent.ACTION_DOWN, 20, 20)); fence.attach(sink)
            router.accept(packet(MotionEvent.ACTION_UP, 21, 20))
            check(freezes == 1 && sink.downs.isEmpty() && sink.finished.isEmpty()); cases++
        }
        run {
            val fence = StreamInputFence({ _, _ -> true }, {})
            val router = CanvasGestureRouter(fence, 8.0); val sink = BoundarySink()
            router.accept(packet(MotionEvent.ACTION_DOWN, 10, 10)); router.accept(packet(MotionEvent.ACTION_MOVE, 11, 10))
            fence.attach(sink)
            router.accept(packet(MotionEvent.ACTION_MOVE, 12, 10)); router.accept(packet(MotionEvent.ACTION_UP, 13, 10))
            check(sink.downs.single().timeMs == 10L && sink.extended.map { it.timeMs } == listOf(11L, 12L) &&
                sink.finished.single().timeMs == 13L); cases++
        }
        run {
            val fence = StreamInputFence({ _, _ -> true }, {})
            val router = CanvasGestureRouter(fence, 8.0); val sink = BoundarySink()
            router.accept(packet(MotionEvent.ACTION_DOWN, 10, 10)); router.accept(packet(MotionEvent.ACTION_UP, 11, 10))
            fence.interrupt() // Real lifecycle/viewport interruption discards held coordinates.
            router.accept(packet(MotionEvent.ACTION_DOWN, 20, 20)); fence.attach(sink)
            check(sink.downs.isEmpty()); cases++
        }

        val initial = CaptureStreamSignal(1uL, 3uL, 7uL, 1uL, CaptureStreamStage.Live, 8u, 8u, 256uL, null,
            sourceAvailable = true, sourceName = "Owned source")
        run {
            val last = initial.copy(sequence = 2uL, frameSequence = 99uL)
            val published = mutableListOf<CaptureStreamSignal>()
            val terminal = observeStreamSignals(initial, flow { emit(last); throw SessionFailure(SessionFailureKind.Closed) }, published::add)
            check(published.size == 2 && terminal == last.copy(stage = CaptureStreamStage.Closed, sourceAvailable = false))
            check(published.last().sourceName == initial.sourceName && published.last().connectionEpoch == 3uL); cases++
        }
        run {
            var readAfterClose = false
            val published = mutableListOf<CaptureStreamSignal>()
            val terminal = observeStreamSignals(initial, flow {
                emit(initial.copy(stage = CaptureStreamStage.Closed, sourceAvailable = true))
                readAfterClose = true; error("must not poll a closed receiver")
            }, published::add)
            check(!readAfterClose && terminal?.stage == CaptureStreamStage.Closed && !published.single().sourceAvailable); cases++
        }
        run {
            val published = mutableListOf<CaptureStreamSignal>()
            val failure = runCatching { observeStreamSignals(initial, flow { throw SessionFailure(SessionFailureKind.Storage) }, published::add) }.exceptionOrNull()
            check(failure is SessionFailure && failure.kind == SessionFailureKind.Storage && published.isEmpty()); cases++
        }
        run {
            val entered = CompletableDeferred<Unit>(); val published = mutableListOf<CaptureStreamSignal>()
            val observer = launch(start = CoroutineStart.UNDISPATCHED) {
                observeStreamSignals(initial, flow { entered.complete(Unit); awaitCancellation() }, published::add)
            }
            entered.await(); observer.cancelAndJoin()
            check(observer.isCancelled && published.isEmpty()); cases++
        }
        run {
            var callbacks = 0
            val failure = runCatching {
                observeStreamSignals(initial, flowOf(initial)) { callbacks++; throw SessionFailure(SessionFailureKind.Closed) }
            }.exceptionOrNull()
            check(failure is SessionFailure && failure.kind == SessionFailureKind.Closed && callbacks == 1); cases++
        }
        run {
            val published = mutableListOf<CaptureStreamSignal>()
            check(observeStreamSignals(initial, flowOf(initial), published::add) == null)
            check(published.single().stage == CaptureStreamStage.Live); cases++
        }
        cases
    }
}

private class BoundarySink : CanvasGestureSink {
    val downs = mutableListOf<PenSample>(); val extended = mutableListOf<PenSample>(); val finished = mutableListOf<PenSample>()
    override fun beginTool(sample: PenSample, capabilities: StylusCapabilities) { downs += sample }
    override fun extendTool(samples: List<PenSample>) { extended += samples }
    override fun finishTool(samples: List<PenSample>) { finished += samples }
    override fun cancelTool() { }
    override fun pan(dx: Double, dy: Double) { }
    override fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double) { }
    override fun undo() { }
    override fun redo() { }
    override fun contextMenu(x: Double, y: Double) { }
    override fun hover(sample: PenSample?, capabilities: StylusCapabilities) { }
}
