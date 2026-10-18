package com.visualworkbench.android.editor

import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.Checkbox
import androidx.compose.material.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp

/** Root-owned snap preference, read on every real rotation sample. */
@Composable
internal fun RotationSnapControl(checked: Boolean, enabled: Boolean, changed: (Boolean) -> Unit,
    modifier: Modifier = Modifier) {
    Row(modifier.heightIn(min = 48.dp).toggleable(value = checked, enabled = enabled,
        role = Role.Checkbox, onValueChange = changed), verticalAlignment = Alignment.CenterVertically) {
        Checkbox(checked = checked, onCheckedChange = null, enabled = enabled)
        Text("Snap rotation to 15°")
    }
}
