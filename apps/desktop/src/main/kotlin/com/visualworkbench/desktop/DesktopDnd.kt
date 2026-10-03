package com.visualworkbench.desktop

import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.awt.SwingPanel
import java.awt.Component
import java.awt.EventQueue
import java.awt.SecondaryLoop
import java.awt.Toolkit
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.Transferable
import java.awt.datatransfer.UnsupportedFlavorException
import java.awt.dnd.*
import java.io.File
import java.nio.file.Path
import java.util.Collections
import javax.swing.JButton
import javax.swing.Timer

/** AWT completes an uncompleted native drop when drop() returns. Keep that
 * callback on the stack while pumping events, without blocking file IO on EDT.
 * Timeout/disposal rejects this drop and requests cooperative cancellation; it
 * never deletes staging still owned by its producer. */
internal class DropStagingWait(private val timeoutMs: Int = 15_000, private val timedOut: () -> Unit = {}) : AutoCloseable {
    private var result: Boolean? = null
    private var loop: SecondaryLoop? = null
    private var cancel: () -> Unit = {}
    private var used = false
    fun await(start: ((Boolean) -> Unit) -> (() -> Unit)): Boolean {
        check(EventQueue.isDispatchThread() && !used)
        require(timeoutMs in 1..30_000)
        used = true
        val wait = Toolkit.getDefaultToolkit().systemEventQueue.createSecondaryLoop()
        loop = wait
        val timer = Timer(timeoutMs) { if (result == null) { timedOut(); close() } }.apply { isRepeats = false }
        try {
            cancel = start(::complete)
            // A synchronous callback may already have completed. Otherwise all
            // completion/exit mutations run on EDT, so none can slip between
            // this check and SecondaryLoop.enter().
            if (result == null) { timer.start(); wait.enter(); if (result == null) close() }
            return result == true
        } catch (error: Throwable) { close(); throw error
        } finally { timer.stop(); loop = null }
    }
    private fun onEdt(block: () -> Unit) { if (EventQueue.isDispatchThread()) block() else EventQueue.invokeLater(block) }
    private fun complete(copied: Boolean) = onEdt {
        if (result == null) { result = copied; loop?.exit() }
    }
    override fun close() = onEdt {
        if (result == null) {
            result = false
            try { cancel() } finally { loop?.exit() }
        }
    }
}

/** Native file lists only. Bitmap/text flavors are deliberately not requested:
 * image paste goes through the bounded PNG/DIB host service after a user action. */
internal class DesktopDropTarget(component: Component, private val editor: EditorController) : AutoCloseable {
    private val previous = component.dropTarget
    private var activeDrop: DropStagingWait? = null
    private val target = DropTarget(component, DnDConstants.ACTION_COPY, object : DropTargetAdapter() {
        private fun allowed(): Boolean = editor.state.value.let { !it.busy && !it.exportOpen && !it.pasteOpen && it.textAnchor == null }
        private fun accept(event: DropTargetDragEvent) {
            if (allowed() && event.isDataFlavorSupported(DataFlavor.javaFileListFlavor)) event.acceptDrag(DnDConstants.ACTION_COPY)
            else event.rejectDrag()
        }
        override fun dragEnter(event: DropTargetDragEvent) = accept(event)
        override fun dragOver(event: DropTargetDragEvent) = accept(event)
        override fun dropActionChanged(event: DropTargetDragEvent) = accept(event)
        override fun drop(event: DropTargetDropEvent) {
            if (!allowed() || !event.isDataFlavorSupported(DataFlavor.javaFileListFlavor)) { event.rejectDrop(); return }
            event.acceptDrop(DnDConstants.ACTION_COPY)
            var completed = false
            fun finish(copied: Boolean) { if (!completed) { completed = true; event.dropComplete(copied) } }
            try {
                val files = event.transferable.getTransferData(DataFlavor.javaFileListFlavor) as? List<*>
                if (files == null || files.size != 1 || files.single() !is File) {
                    finish(false); editor.report("Drop one local image or MP4 file at a time."); return
                }
                val paths = listOf((files.single() as File).toPath())
                val wait = DropStagingWait(timedOut = { editor.report("Drop staging timed out. No successful drop was acknowledged; any accepted private recovery copy is preserved.") })
                activeDrop = wait
                val copied = try { wait.await { complete ->
                    val job = editor.acceptDrop(paths, complete)
                    val cancel: () -> Unit = { job?.cancel(); Unit }
                    cancel
                } } finally { activeDrop = null }
                // Complete exactly once, on the original native drop callback,
                // after private bytes and recovery ownership are flushed.
                finish(copied)
            } catch (_: Exception) { finish(false); editor.report("The dropped file could not be read. Any retained recovery copy remains available.") }
        }
    }, true)
    override fun close() {
        activeDrop?.close()
        target.isActive = false
        val component = target.component
        if (component?.dropTarget === target) component.dropTarget = previous
        target.component = null
    }
}

internal class PngFileTransferable(path: Path) : Transferable {
    private val files: List<File> = Collections.singletonList(path.toFile())
    override fun getTransferDataFlavors(): Array<DataFlavor> = arrayOf(DataFlavor.javaFileListFlavor)
    override fun isDataFlavorSupported(flavor: DataFlavor): Boolean = flavor == DataFlavor.javaFileListFlavor
    override fun getTransferData(flavor: DataFlavor): Any {
        if (!isDataFlavorSupported(flavor)) throw UnsupportedFlavorException(flavor)
        return files
    }
}

/** The gesture callback never waits for export. It consumes one already-owned
 * lease and starts the actual AWT drag in the callback that supplied the event. */
private class FileDragButton(private val editor: EditorController) : AutoCloseable {
    val button = JButton("Drag prepared PNG")
    var dragging = false
        private set
    private val source = DragSource()
    private val recognizer = source.createDefaultDragGestureRecognizer(button, DnDConstants.ACTION_COPY) { event ->
        val lease = editor.takePreparedDrag()
        if (lease == null) { editor.report("Prepare a PNG drag for the current revision and settings first."); return@createDefaultDragGestureRecognizer }
        dragging = true
        try {
            event.startDrag(DragSource.DefaultCopyDrop, PngFileTransferable(lease.path), object : DragSourceAdapter() {
                override fun dragDropEnd(event: DragSourceDropEvent) {
                    dragging = false
                    button.isEnabled = false
                    lease.close()
                    editor.report(if (event.dropSuccess) "File drag completed. Review it in the destination before sending." else "File drag cancelled. Prepare it again to retry.")
                }
            })
        } catch (_: Exception) { dragging = false; button.isEnabled = false; lease.close(); editor.report("Windows could not start the file drag. Prepare it again to retry.") }
    }
    init {
        button.toolTipText = "Drag the completed PNG file into another app. No text or Send key is attached."
        button.accessibleContext.accessibleDescription = button.toolTipText
    }
    override fun close() {
        // An in-flight DragSource listener retains its lease until dragDropEnd.
        // Removing this recognizer prevents new gestures without dropping it.
        recognizer.component = null
    }
}

@Composable
internal fun NativeFileDragButton(editor: EditorController, enabled: Boolean, modifier: Modifier = Modifier) {
    val holder = remember(editor) { FileDragButton(editor) }
    DisposableEffect(holder) { onDispose { holder.close() } }
    SwingPanel(factory = { holder.button }, modifier = modifier, update = { it.isEnabled = enabled || holder.dragging })
}
