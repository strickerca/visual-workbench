package com.visualworkbench.penprobe

import android.app.Activity
import android.os.Bundle
import android.os.Build
import android.view.KeyEvent
import android.view.View
import android.view.WindowManager
import android.view.WindowInsets
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.LinearLayout
import android.widget.Spinner
import android.widget.TextView
import java.io.File
import java.util.concurrent.Executors
import org.json.JSONObject

class ProbeActivity : Activity() {
    val recorder = TraceRecorder()
    lateinit var surface: ProbeSurface
        private set
    private lateinit var status: TextView
    private lateinit var choice: Spinner
    private lateinit var start: Button
    private lateinit var save: Button
    private val writer = Executors.newSingleThreadExecutor()
    private var busy = false
    private var lastUpdateNs = 0L
    @Volatile var focused = false
        private set

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val layout = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setBackgroundColor(-1) }
        layout.setOnApplyWindowInsetsListener { view, insets ->
            @Suppress("DEPRECATION")
            val padding = if (Build.VERSION.SDK_INT >= 30) insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout()) else insets.systemWindowInsets
            view.setPadding(padding.left, padding.top, padding.right, padding.bottom)
            insets
        }
        val title = TextView(this).apply { text = "Pen probe · T0.04"; textSize = 22f; setPadding(20, 12, 20, 4) }
        layout.addView(title)
        status = TextView(this).apply {
            minLines = 2
            maxLines = 2
            ellipsize = android.text.TextUtils.TruncateAt.END
            text = "Choose a trace, tap Record, then draw below. Save before switching apps."
            setPadding(20, 4, 20, 4)
        }
        layout.addView(status)
        choice = Spinner(this).apply {
            adapter = ArrayAdapter(this@ProbeActivity, android.R.layout.simple_spinner_dropdown_item, TRACE_NAMES)
        }
        layout.addView(choice)
        val controls = LinearLayout(this)
        start = Button(this).apply { text = "Record"; setOnClickListener {
            if (surface.width > 0 && !busy) {
                surface.clearInk()
                recorder.start(choice.selectedItem.toString(), "owner", surface.width, surface.height)
                showState("Recording. Draw only test shapes or the word ink.")
            }
        } }
        save = Button(this).apply { text = "Save trace"; isEnabled = false; setOnClickListener { saveTrace("saved") } }
        controls.addView(start, LinearLayout.LayoutParams(0, -2, 1f))
        controls.addView(save, LinearLayout.LayoutParams(0, -2, 1f))
        layout.addView(controls)
        surface = ProbeSurface(this, recorder) {
            if (System.nanoTime() - lastUpdateNs > 200_000_000L) {
                lastUpdateNs = System.nanoTime()
                showState(if (recorder.recording) "Recording: ${recorder.count()} samples" else "Recording stopped; save the incomplete trace.")
            }
        }.apply { contentDescription = "Pen recording canvas"; isFocusable = true }
        layout.addView(surface, LinearLayout.LayoutParams(-1, 0, 1f))
        setContentView(layout)
    }

    private fun showState(message: String) {
        status.text = message
        start.isEnabled = !busy && !recorder.recording
        save.isEnabled = !busy && (recorder.recording || recorder.count() > 0)
        choice.isEnabled = !busy && !recorder.recording
    }

    private fun saveTrace(reason: String) {
        if (busy) return
        val previous = recorder.snapshot().getString("stop_reason")
        recorder.stop(if (previous == "recording") reason else previous)
        val snapshot = recorder.snapshot()
        busy = true
        showState("Saving trace…")
        writer.execute {
            val result = runCatching { writeSnapshot(snapshot) }
            runOnUiThread {
                busy = false
                showState(result.fold({ "Saved ${it.name}. Choose the next trace." }, { "Save failed. Tap Save trace to retry." }))
                if (result.isSuccess && choice.selectedItemPosition < TRACE_NAMES.lastIndex) choice.setSelection(choice.selectedItemPosition + 1)
            }
        }
    }

    /** Only app-private files. Caller runs off the main thread; no external/storage permission. */
    fun writeSnapshot(snapshot: JSONObject): File {
        val directory = File(filesDir, "traces").apply { check(isDirectory || mkdirs()) }
        val file = File(directory, "${snapshot.getString("name")}-${java.util.UUID.randomUUID()}.json")
        check(file.createNewFile())
        try { file.outputStream().buffered().use { it.write(snapshot.toString().toByteArray(Charsets.UTF_8)) } }
        catch (error: Exception) { file.delete(); throw error }
        return file
    }

    override fun dispatchKeyEvent(event: KeyEvent): Boolean {
        recorder.key(event)
        if (event.keyCode in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F8) {
            showState("Air Action key F${event.keyCode - KeyEvent.KEYCODE_F1 + 1}, action ${event.action}")
            return true
        }
        return super.dispatchKeyEvent(event)
    }
    override fun onWindowFocusChanged(hasFocus: Boolean) { super.onWindowFocusChanged(hasFocus); focused = hasFocus }
    override fun onPause() {
        if (recorder.recording) saveTrace("interrupted_background")
        super.onPause()
    }
    override fun onDestroy() { writer.shutdown(); super.onDestroy() }

    companion object {
        val TRACE_NAMES = arrayOf(
            "line-slow-01", "line-slow-02", "line-fast-01", "line-fast-02", "line-fast-03",
            "circle-01", "circle-02", "circle-03", "word-ink-01", "word-ink-02", "word-ink-03",
            "flick-01", "flick-02", "flick-03", "hover-01", "hover-02", "palm-01", "palm-02",
            "button-01", "button-02", "tilt-right-01", "tilt-right-02", "tilt-bottom-01", "tilt-bottom-02",
            "air-command-on", "air-command-off", "air-actions", "eraser", "thermal"
        )
    }
}
