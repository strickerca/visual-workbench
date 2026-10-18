package com.visualworkbench.shared

public enum class AirActionGesture(public val label: String) {
    SingleClick("Single click"), DoubleClick("Double click"),
    SwipeLeft("Swipe left"), SwipeRight("Swipe right"),
    SwipeUp("Swipe up"), SwipeDown("Swipe down"),
    CircleClockwise("Circle clockwise"), CircleCounterclockwise("Circle counterclockwise"),
}

/** Deliberately closed: no Send, Submit, Apply, grant or arbitrary shortcut. */
public enum class AirActionCommand(public val label: String) {
    TogglePrecisionLens("Toggle precision lens"), FitRestoreView("Fit / restore view"),
    Undo("Undo"), Redo("Redo"), PreviousTool("Previous tool"), NextTool("Next tool"),
    LensMagnificationUp("Increase lens magnification"), LensMagnificationDown("Decrease lens magnification"),
}

/** Missing entries use the specification default; an explicit null disables a gesture.
 * The caller persists stable enum names using its existing preference owner. */
public class AirActionBindings(overrides: Map<AirActionGesture, AirActionCommand?> = emptyMap()) {
    private val commands: Map<AirActionGesture, AirActionCommand?> = defaults + overrides
    public fun command(gesture: AirActionGesture): AirActionCommand? = commands[gesture]
    public fun remap(gesture: AirActionGesture, command: AirActionCommand?): AirActionBindings =
        AirActionBindings(commands + (gesture to command))
    public fun snapshot(): Map<AirActionGesture, AirActionCommand?> = commands.toMap()
    override fun equals(other: Any?): Boolean = other is AirActionBindings && commands == other.commands
    override fun hashCode(): Int = commands.hashCode()

    private companion object {
        val defaults = mapOf(
            AirActionGesture.SingleClick to AirActionCommand.TogglePrecisionLens,
            AirActionGesture.DoubleClick to AirActionCommand.FitRestoreView,
            AirActionGesture.SwipeLeft to AirActionCommand.Undo,
            AirActionGesture.SwipeRight to AirActionCommand.Redo,
            AirActionGesture.SwipeUp to AirActionCommand.PreviousTool,
            AirActionGesture.SwipeDown to AirActionCommand.NextTool,
            AirActionGesture.CircleClockwise to AirActionCommand.LensMagnificationUp,
            AirActionGesture.CircleCounterclockwise to AirActionCommand.LensMagnificationDown,
        )
    }
}

public enum class AirActionSupport { Unknown, Supported, Unsupported }

/** Settings/connection/interference observations must come from the existing host
 * or the owner's check. Receiving F1-F8 alone cannot prove BLE or hardware support. */
public data class AirActionSetup(
    public val support: AirActionSupport = AirActionSupport.Unknown,
    public val enabled: Boolean? = null,
    public val connected: Boolean? = null,
    public val airCommandInterference: Boolean? = null,
    public val keyDeliveryObserved: Boolean = false,
) {
    public val routingReady: Boolean get() = support == AirActionSupport.Supported &&
        enabled == true && connected == true && airCommandInterference == false
    public val message: String get() = when {
        support == AirActionSupport.Unsupported -> "Air Actions are unavailable. Use the on-screen controls below."
        support == AirActionSupport.Unknown -> "Air Actions support is unconfirmed. S23 Ultra support needs an actual device check."
        enabled == false -> "Enable Visual Workbench in Settings > Advanced features > S Pen > Air actions."
        connected == false -> "Reconnect the Bluetooth S Pen in Air actions settings. On-screen controls remain available."
        airCommandInterference == true -> "In S Pen settings, disable the Air Command option that opens Air Command with the pen button. Then check again."
        !routingReady -> "Check Air Actions is enabled, the S Pen is connected, and its button does not open Air Command."
        !keyDeliveryObserved -> "Setup is confirmed; key delivery is still unverified. Try each gesture while this app is in front."
        else -> "Action key delivery was observed. Physical S Pen and Bluetooth acceptance still needs the device check."
    }
}

public enum class AirActionContext { Canvas, RemoteApp, Dialog }

/** Local admission ticket, never an input grant. Increment epoch on every
 * project/document/page/target/grant/context/focus/lifecycle replacement. */
public data class AirActionBinding(public val epoch: Long, public val context: AirActionContext)

public enum class AirActionKeyPhase { Down, Up, Multiple }

/** An ephemeral stream number disambiguates simultaneous input sources. Never
 * persist or log a platform device identifier. Times use the host monotonic clock. */
public data class AirActionKeyEvent(
    public val gesture: AirActionGesture,
    public val phase: AirActionKeyPhase,
    public val stream: Int,
    public val downTimeMs: Long,
    public val eventTimeMs: Long,
    public val repeatCount: Int = 0,
    public val canceled: Boolean = false,
)
