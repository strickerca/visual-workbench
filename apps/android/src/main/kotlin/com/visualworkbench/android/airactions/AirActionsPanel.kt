package com.visualworkbench.android.airactions

import android.content.Context
import android.view.View
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Spinner
import android.widget.TextView
import com.visualworkbench.shared.AirActionBindings
import com.visualworkbench.shared.AirActionCommand
import com.visualworkbench.shared.AirActionGesture
import com.visualworkbench.shared.AirActionSetup
import com.visualworkbench.shared.AirActionSupport

/** Callback-only settings/fallback surface. Embed with AndroidView or an owned
 * dialog. The existing controller persists remaps and checks each command again.
 * Setup observations are explicit; this view never reads private vendor settings,
 * identifies a device, opens Settings, grants permission or infers BLE delivery. */
internal class AirActionsPanel(context: Context) : ScrollView(context) {
    private val content = LinearLayout(context).apply { orientation = LinearLayout.VERTICAL }
    private val minimumTarget = (48 * resources.displayMetrics.density).toInt()
    init { addView(content) }

    fun render(setup: AirActionSetup, bindings: AirActionBindings,
        commandEnabled: (AirActionCommand) -> Boolean,
        onSetup: (AirActionSetup) -> Unit,
        onRemap: (AirActionGesture, AirActionCommand?) -> Unit,
        onCommand: (AirActionCommand) -> Unit) {
        content.removeAllViews()
        text("Air Actions")
        text(setup.message)
        text("In Settings > Advanced features > S Pen > Air actions, enable this app and check the pen connection. In S Pen settings, disable the option that opens Air Command with the pen button.")
        text("Confirm these observations after checking Settings and trying the button. A received action key alone does not identify an S Pen. Recheck after reconnecting.")
        choice("Device support", AirActionSupport.entries.map { it.name }, setup.support.ordinal) {
            onSetup(setup.copy(support = AirActionSupport.entries[it], keyDeliveryObserved = false))
        }
        observed("Air Actions enabled", setup.enabled) { onSetup(setup.copy(enabled = it, keyDeliveryObserved = false)) }
        observed("Bluetooth S Pen connected", setup.connected) { onSetup(setup.copy(connected = it, keyDeliveryObserved = false)) }
        observed("Pen button opens Air Command", setup.airCommandInterference) {
            onSetup(setup.copy(airCommandInterference = it, keyDeliveryObserved = false))
        }
        text("Gesture commands stay separate from Send, Submit and Apply. Barrel hold/peek and barrel click keep their existing behavior.")
        text("These gesture names assume the defaults in Samsung Air actions settings. Restore those defaults there before remapping gestures here.")
        val commands = AirActionCommand.entries
        for (gesture in AirActionGesture.entries) {
            val selected = bindings.command(gesture)
            choice(gesture.label, listOf("Disabled") + commands.map { it.label },
                if (selected == null) 0 else commands.indexOf(selected) + 1) {
                onRemap(gesture, if (it == 0) null else commands[it - 1])
            }
        }
        text("On-screen equivalents")
        // Present every safe command even when Air Actions is unavailable.
        for (command in commands) content.addView(Button(context).apply {
            text = command.label
            minimumHeight = minimumTarget
            isEnabled = commandEnabled(command)
            setOnClickListener { onCommand(command) }
        })
    }

    private fun text(value: String) { content.addView(TextView(context).apply { text = value }) }

    private fun observed(label: String, value: Boolean?, changed: (Boolean?) -> Unit) {
        choice(label, listOf("Not checked", "Yes", "No"), when (value) { null -> 0; true -> 1; false -> 2 }) {
            changed(when (it) { 0 -> null; 1 -> true; else -> false })
        }
    }

    private fun choice(label: String, values: List<String>, selection: Int, changed: (Int) -> Unit) {
        text(label)
        content.addView(Spinner(context).apply {
            contentDescription = label
            minimumHeight = minimumTarget
            adapter = ArrayAdapter(context, android.R.layout.simple_spinner_dropdown_item, values)
            setSelection(selection, false)
            // Ignore the initial callback, including the asynchronous layout callback.
            var previous = selection
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onNothingSelected(parent: AdapterView<*>?) = Unit
                override fun onItemSelected(parent: AdapterView<*>?, view: View?, position: Int, id: Long) {
                    if (position != previous) { previous = position; changed(position) }
                }
            }
        })
    }
}
