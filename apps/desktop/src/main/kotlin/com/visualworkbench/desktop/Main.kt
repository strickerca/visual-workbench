package com.visualworkbench.desktop

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Surface
import androidx.compose.material.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import com.visualworkbench.shared.WorkbenchStartup
import java.nio.file.Files

private fun loadNativeSmokeLibraries() {
    val directory = Files.createTempDirectory("visual-workbench-native-")
    for (name in listOf("vw_core.dll", "vw_host.dll")) {
        val resource = checkNotNull(WorkbenchStartup::class.java.getResourceAsStream("/native/$name")) {
            "Native build artifact is missing: $name"
        }
        val destination = directory.resolve(name)
        resource.use { Files.copy(it, destination) }
        System.load(destination.toAbsolutePath().toString())
    }
    println("VW_NATIVE_LIBRARIES_LOADED count=2")
}

fun main() {
    loadNativeSmokeLibraries()
    application {
    Window(onCloseRequest = ::exitApplication, title = WorkbenchStartup.title,
        state = rememberWindowState(width = 880.dp, height = 600.dp)) {
        val density = LocalDensity.current.density
        LaunchedEffect(density) {
            val transform = window.graphicsConfiguration.defaultTransform
            println("VW_DESKTOP_READY composeDensity=$density awtScaleX=${transform.scaleX} awtScaleY=${transform.scaleY}")
        }
        MaterialTheme {
            Surface(Modifier.fillMaxSize()) {
                Column(Modifier.padding(36.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                    Text(WorkbenchStartup.title, style = MaterialTheme.typography.h3)
                    Text(WorkbenchStartup.stage, style = MaterialTheme.typography.h6)
                    Text(WorkbenchStartup.message)
                    Text("Windows + Kotlin + Compose + Rust")
                    Text("Display density: $density")
                }
            }
        }
    }
}
}
