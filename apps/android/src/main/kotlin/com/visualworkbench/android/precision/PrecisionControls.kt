package com.visualworkbench.android.precision

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.PrecisionController
import kotlin.math.roundToInt

@Composable internal fun PrecisionControls(controller:PrecisionController) {
    val state by controller.state.collectAsState()
    Column {
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
            TextButton(onClick=controller::toggle,modifier=Modifier.heightIn(min=48.dp)){Text(if(state.frame.lens!=null)"Close precision lens" else "Precision lens")}
            listOf(2,4,8).forEach { value -> TextButton(onClick={controller.magnification(value)},modifier=Modifier.heightIn(min=48.dp)){Text(if(state.magnification==value)"✓ ${value}×" else "${value}×")} }
            TextButton(onClick={controller.inspection(!state.inspection)},modifier=Modifier.heightIn(min=48.dp)){Text(if(state.inspection)"Loupe on" else "Loupe off")}
            TextButton(onClick={controller.smooth(!state.smooth)},modifier=Modifier.heightIn(min=48.dp)){Text(if(state.smooth)"Smooth" else "Pixel grid")}
            TextButton(onClick={controller.leftHanded(!state.leftHanded)},modifier=Modifier.heightIn(min=48.dp)){Text(if(state.leftHanded)"Left-handed" else "Right-handed")}
            TextButton(onClick=controller::resetLoupePosition,modifier=Modifier.heightIn(min=48.dp)){Text("Reset loupe position")}
        }
        if(state.frame.lens!=null)Row(Modifier.fillMaxWidth().padding(horizontal=12.dp)){Text("Lens size");Slider(state.sizeFraction.toFloat(),{controller.size(it.toDouble().coerceIn(.18,.60))},valueRange=.18f.. .60f,modifier=Modifier.weight(1f).heightIn(min=48.dp))}
        state.message?.let{Text(it,Modifier.padding(horizontal=12.dp),style=MaterialTheme.typography.caption)}
    }
}
/** Only visible inset headers receive dragging; the remaining pane is the actual
 * canvas input surface. A drag owns its pointer and cannot also create a stroke. */
@Composable internal fun BoxScope.PrecisionHandles(controller:PrecisionController) {
    val state by controller.state.collectAsState();val density=LocalDensity.current
    listOfNotNull(state.frame.lens,state.frame.loupe).forEach { pane ->
        val r=pane.rectangle
        Text(if(pane.editable)"Precision ${pane.magnification}× · drag" else "Inspect 4× · drag",
            Modifier.offset{IntOffset(r.x.roundToInt(),r.y.roundToInt())}.width(with(density){r.width.toFloat().toDp()})
                .heightIn(min=48.dp).background(MaterialTheme.colors.surface.copy(alpha=.85f))
                .pointerInput(controller,pane.editable){detectDragGestures{change,amount->change.consume();if(pane.editable)controller.moveLens(amount.x.toDouble(),amount.y.toDouble())else controller.moveLoupe(amount.x.toDouble(),amount.y.toDouble())}}
                .padding(8.dp),style=MaterialTheme.typography.caption)
    }
}
