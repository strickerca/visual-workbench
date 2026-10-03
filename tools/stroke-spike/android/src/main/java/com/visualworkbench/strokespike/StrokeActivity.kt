package com.visualworkbench.strokespike

import android.app.Activity
import android.app.AlertDialog
import android.os.Bundle
import android.view.View
import android.view.WindowManager
import android.widget.Button
import android.widget.FrameLayout
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.EditText
import java.security.SecureRandom

/** Blind labels are stable for this comparison session; preferences are never inferred. */
class StrokeActivity : Activity() {
    internal val metrics = StrokeMetrics()
    internal lateinit var rust: RustInkSurface
    internal lateinit var jetpack: JetpackInkSurface
    private lateinit var canvas: FrameLayout
    private lateinit var status: TextView
    private var selected = 1
    private var firstIsRust = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val preferences = getPreferences(MODE_PRIVATE)
        firstIsRust = savedInstanceState?.getBoolean("firstIsRust") ?: if (preferences.contains("firstIsRust"))
            preferences.getBoolean("firstIsRust", false) else SecureRandom().nextBoolean().also {
                preferences.edit().putBoolean("firstIsRust", it).apply()
            }
        selected = savedInstanceState?.getInt("selected", 1) ?: 1
        rust = RustInkSurface(this, metrics)
        jetpack = JetpackInkSurface(this, metrics)
        val column = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        column.addView(TextView(this).apply {
            text = "Ink comparison\nDraw handwriting, circles and fast lines in both demos. OnePlus tests cover software; S Pen feel and display latency require the S23."
            textSize = 16f; setPadding(20, 12, 20, 12)
        })
        val buttons = LinearLayout(this)
        fun button(label: String, action: () -> Unit) {
            buttons.addView(Button(this).apply { text = label; setOnClickListener { action() } },
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        }
        button("Demo 1") { selectDemo(1) }
        button("Demo 2") { selectDemo(2) }
        button("Clear") { rust.clearInk(); jetpack.clearInk() }
        column.addView(buttons)
        status = TextView(this).apply { textSize = 16f; setPadding(20, 8, 20, 8) }
        column.addView(status)
        column.addView(Button(this).apply { text = "Record comparison preference"; setOnClickListener { recordPreference() } })
        canvas = FrameLayout(this)
        // Jetpack Ink 1.0's Android 11 helper retains its original render-thread
        // identity. Keep the two engines attached while switching visibility;
        // detaching and reattaching the same authoring view creates a new render
        // thread and trips that helper's thread-affinity assertion.
        canvas.addView(rust, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        canvas.addView(jetpack, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        column.addView(canvas, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        setContentView(column)
        selectDemo(selected)
    }

    private fun selectDemo(number: Int) {
        selected = number
        val selectedView = if ((number == 1) == firstIsRust) rust else jetpack
        showSurface(selectedView)
        status.text = "Demo $number · comparison canvas"
    }
    private fun recordPreference() {
        val types = arrayOf("Handwriting", "Circles", "Fast lines")
        AlertDialog.Builder(this).setTitle("Stroke type").setItems(types) { _, type ->
            val choices = arrayOf("Demo 1", "Demo 2", "No preference")
            AlertDialog.Builder(this).setTitle("Which feels better for ${types[type].lowercase()}?").setItems(choices) { _, choice ->
                val comments = EditText(this).apply { hint = "Why? (optional)"; maxLines = 4 }
                AlertDialog.Builder(this).setTitle("${types[type]}: ${choices[choice]}").setView(comments)
                    .setPositiveButton("Save") { _, _ ->
                        getPreferences(MODE_PRIVATE).edit().putString("preference_$type", choices[choice])
                            .putString("comment_$type", comments.text.toString().take(2000)).apply()
                        status.text = "${types[type]} preference saved on this phone."
                    }.setNegativeButton("Cancel", null).show()
            }.setNegativeButton("Cancel", null).show()
        }.setNegativeButton("Cancel", null).show()
    }
    internal fun showSurface(surface: View) {
        require(surface === rust || surface === jetpack)
        rust.cancelActive(); jetpack.cancelActive()
        rust.visibility = if (surface === rust) View.VISIBLE else View.INVISIBLE
        jetpack.visibility = if (surface === jetpack) View.VISIBLE else View.INVISIBLE
    }
    override fun onSaveInstanceState(outState: Bundle) {
        outState.putBoolean("firstIsRust", firstIsRust); outState.putInt("selected", selected)
        super.onSaveInstanceState(outState)
    }
    override fun onPause() { rust.cancelActive(); jetpack.cancelActive(); super.onPause() }
}
