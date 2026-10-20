package com.visualworkbench.desktop.stream

import java.awt.ActiveEvent
import java.awt.AWTEvent
import java.awt.Component
import java.awt.EventQueue
import java.awt.Rectangle
import java.awt.Toolkit
import java.awt.event.InvocationEvent
import java.awt.event.PaintEvent
import java.util.concurrent.atomic.AtomicLong

/** D3D restoration posts Canvas paint/replacement events even when it restores
 * the lost flag before drawImage returns. Keep a scoped event of each relevant
 * ID queued during this synchronous EDT operation, so EventQueue calls this
 * Canvas's coalescing hook for every subsequent restoration request.
 *
 * EventQueue.postEvent first flushes SunToolkit's pending native-event queue.
 * Therefore current() observes requests posted inside drawImage/sync without
 * dispatching a repaint or installing a process-global EventQueue hook. */
internal class StreamPresentationEpoch(private val component:Component) {
    private val epoch=AtomicLong()
    private val flushSource=Any()
    private interface Marker
    private class PaintMarker(component:Component):PaintEvent(component,PAINT,Rectangle()),ActiveEvent,Marker {
        override fun dispatch()=Unit
    }
    private class InvocationMarker(component:Component):InvocationEvent(component,Runnable{}),Marker
    fun coalescing(event:AWTEvent){
        if(event.source===component&&event !is Marker&&(event is PaintEvent||event is InvocationEvent))epoch.incrementAndGet()
    }
    fun begin():StreamDestinationState {
        check(EventQueue.isDispatchThread()&&!component.ignoreRepaint&&!Thread.currentThread().isInterrupted)
        val queue=Toolkit.getDefaultToolkit().systemEventQueue
        queue.postEvent(PaintMarker(component))
        queue.postEvent(InvocationMarker(component))
        val captured=epoch.get()
        return StreamDestinationState {
            if(!EventQueue.isDispatchThread()||component.ignoreRepaint||Thread.currentThread().isInterrupted)false
            else {
                // A separate source prevents the flush marker from counting
                // as a destination replacement request.
                queue.postEvent(InvocationEvent(flushSource,Runnable{}))
                !Thread.currentThread().isInterrupted&&epoch.get()==captured
            }
        }
    }
}
