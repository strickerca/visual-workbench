package com.visualworkbench.ui

import androidx.compose.ui.graphics.ImageBitmap

/** What the shared AI screens need from an app's own bitmap type: something
 * Compose can draw. Each app keeps ownership and disposal of its pixels. */
internal interface AiDisplayImage {
    val display: ImageBitmap
}
