package com.visualworkbench.android.stream

import com.visualworkbench.android.input.*
import com.visualworkbench.shared.Shape

internal fun runStreamInputContract(): Int {
    val caps = StylusCapabilities(true, false, false, true, false, false)
    val down = PenSample(1, PointerTool.Pen, 2.0, 3.0, 10, 1f, 1f, null, null, 0)
    var cases = 0
    run {
        var freezes = 0
        val fence = StreamInputFence({ _, _ -> true }, { freezes++ })
        fence.hover(down, caps)
        for (tool in listOf(PointerTool.Finger, PointerTool.Eraser, PointerTool.Palm, PointerTool.Mouse))
            fence.beginTool(down.copy(tool = tool), caps)
        check(freezes == 0); cases++
    }
    run {
        var freezes = 0
        val fence = StreamInputFence({ _, _ -> false }, { freezes++ })
        fence.beginTool(down, caps); check(freezes == 0); cases++
    }
    run {
        var freezes = 0
        val fence = StreamInputFence({ _, _ -> true }, { freezes++ })
        val sink = InputSink()
        fence.beginTool(down, caps); fence.extendTool(listOf(down.copy(timeMs = 11)))
        fence.finishTool(listOf(down.copy(timeMs = 12)))
        fence.attach(sink)
        check(freezes == 1 && sink.downs == listOf(down) && sink.finished.map { it.timeMs } == listOf(11L, 12L))
        cases++
    }
    run {
        val fence = StreamInputFence({ _, _ -> true }, {})
        val sink = InputSink()
        fence.beginTool(down, caps); fence.cancelTool(); fence.attach(sink)
        check(sink.downs.isEmpty()); cases++
    }
    run {
        val fence = StreamInputFence({ _, _ -> true }, {})
        val sink = InputSink()
        fence.beginTool(down, caps); fence.extendTool(List(4097) { down }); fence.attach(sink)
        check(sink.downs.isEmpty()); cases++
    }
    run {
        val fence = StreamInputFence({ _, _ -> true }, {})
        val sink = InputSink()
        fence.beginTool(down, caps); fence.finishTool(List(1025) { down.copy(timeMs = 11) }); fence.attach(sink)
        check(sink.maxBatch <= 512 && sink.extended.size + sink.finished.size == 1025); cases++
    }
    run {
        val fence = StreamInputFence({ _, _ -> true }, {})
        val sink = InputSink(); fence.attach(sink)
        fence.beginTool(down, caps); fence.block(true); fence.extendTool(listOf(down))
        check(sink.downs.size == 1 && sink.cancels == 1 && sink.extended.isEmpty()); cases++
    }
    run {
        check(!streamAnnotationShapeSupported(Shape.Result("asset")))
        check(!streamAnnotationShapeSupported(Shape.Adjustment))
        check(!streamAnnotationShapeSupported(Shape.Stroke("vector_eraser")))
        check(streamAnnotationShapeSupported(Shape.Stroke("pen"))); cases++
    }
    return cases
}

private class InputSink : CanvasGestureSink {
    val downs = mutableListOf<PenSample>(); val extended = mutableListOf<PenSample>(); val finished = mutableListOf<PenSample>()
    var cancels = 0
    var maxBatch = 0
    override fun beginTool(sample: PenSample, capabilities: StylusCapabilities) { downs += sample }
    override fun extendTool(samples: List<PenSample>) { maxBatch = maxOf(maxBatch, samples.size); extended += samples }
    override fun finishTool(samples: List<PenSample>) { maxBatch = maxOf(maxBatch, samples.size); finished += samples }
    override fun cancelTool() { cancels++ }
    override fun pan(dx: Double, dy: Double) { }
    override fun pinch(previousX: Double, previousY: Double, nextX: Double, nextY: Double, scale: Double, radians: Double) { }
    override fun undo() { }
    override fun redo() { }
    override fun contextMenu(x: Double, y: Double) { }
    override fun hover(sample: PenSample?, capabilities: StylusCapabilities) { }
}
