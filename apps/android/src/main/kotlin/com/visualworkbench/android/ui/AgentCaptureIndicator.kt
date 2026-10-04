package com.visualworkbench.android.ui

import androidx.compose.foundation.layout.*
import androidx.compose.material.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.AgentCaptureStatus
import com.visualworkbench.shared.agentCaptureStatusText

/** Always in the main app layout, independent of the Pairing/MCP screens. */
@Composable internal fun AgentCaptureIndicator(status: AgentCaptureStatus) {
    val text = agentCaptureStatusText(status)
    Surface(Modifier.fillMaxWidth().semantics { contentDescription = text }, elevation = 4.dp,
        color = if (!status.known || status.activeCapture || status.activeGrantCount != 0u) MaterialTheme.colors.secondary else MaterialTheme.colors.surface) {
        Text(text, Modifier.padding(horizontal = 12.dp, vertical = 8.dp), style = MaterialTheme.typography.caption)
    }
}
