package com.visualworkbench.desktop

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

internal val StreamPanelHeaderHeight=80.dp
internal val StreamPanelIssueHeight=48.dp
internal val StreamPanelToolbarHeight=56.dp

/** Every chrome slot exists before first contact. Freeze, errors and tool
 * arrival cannot shift or resize the canvas underneath that accepted contact. */
@Composable internal fun StreamPanelLayout(modifier:Modifier=Modifier,
    header:@Composable ()->Unit,issue:@Composable ()->Unit,tools:@Composable ()->Unit,
    canvas:@Composable BoxScope.()->Unit){
    Column(modifier){
        Box(Modifier.fillMaxWidth().height(StreamPanelHeaderHeight)){header()}
        Box(Modifier.fillMaxWidth().height(StreamPanelIssueHeight)){issue()}
        Box(Modifier.fillMaxWidth().height(StreamPanelToolbarHeight)){tools()}
        Box(Modifier.weight(1f).fillMaxWidth(),content=canvas)
    }
}
