package com.visualworkbench.android.ui

import androidx.compose.material.MaterialTheme
import androidx.compose.material.darkColors
import androidx.compose.material.lightColors
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import kotlin.math.pow

/** Fixed sRGB palette: document pixels are color managed separately by the core. */
internal object WorkbenchPalette {
    val dark = darkColors(primary = Color(0xff74dad1), primaryVariant = Color(0xff74dad1),
        secondary = Color(0xffc5d4dc), background = Color(0xff0c1217), surface = Color(0xff152129),
        onPrimary = Color(0xff052d2a), onSecondary = Color(0xff132c33), onBackground = Color(0xffecf3f5),
        onSurface = Color(0xffecf3f5), error = Color(0xffffa5ab), onError = Color(0xff3b0713))
    val light = lightColors(primary = Color(0xff006b62), primaryVariant = Color(0xff006b62),
        secondary = Color(0xff445c64), background = Color(0xfff2f5f4), surface = Color(0xffffffff),
        onPrimary = Color.White, onSecondary = Color.White, onBackground = Color(0xff132c33),
        onSurface = Color(0xff132c33), error = Color(0xffa52236), onError = Color.White)

    fun contrast(foreground: Color, background: Color): Double {
        fun linear(value: Float): Double = if (value <= .04045f) value / 12.92 else ((value + .055) / 1.055).pow(2.4)
        fun luminance(color: Color) = .2126 * linear(color.red) + .7152 * linear(color.green) + .0722 * linear(color.blue)
        val a = luminance(foreground); val b = luminance(background)
        return (maxOf(a, b) + .05) / (minOf(a, b) + .05)
    }
}

@Composable
internal fun WorkbenchTheme(dark: Boolean, content: @Composable () -> Unit) {
    // Navigation, selection and rail positioning intentionally use no animations.
    // This also honors reduced motion when requested by Android or the app.
    MaterialTheme(colors = if (dark) WorkbenchPalette.dark else WorkbenchPalette.light, content = content)
}
