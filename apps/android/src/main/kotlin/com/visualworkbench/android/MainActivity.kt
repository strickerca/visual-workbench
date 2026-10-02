package com.visualworkbench.android

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.MaterialTheme
import androidx.compose.material.Surface
import androidx.compose.material.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.visualworkbench.shared.WorkbenchStartup

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Proves that the actual Rust artifact is packaged and loadable on Android.
        System.loadLibrary("vw_core")
        setContent {
            MaterialTheme {
                Surface(Modifier.fillMaxSize()) {
                    Column(Modifier.padding(28.dp), verticalArrangement = Arrangement.spacedBy(18.dp)) {
                        Text(WorkbenchStartup.title, style = MaterialTheme.typography.h4)
                        Text(WorkbenchStartup.stage, style = MaterialTheme.typography.h6)
                        Text(WorkbenchStartup.message)
                        Text("Android + Kotlin + Compose + Rust")
                    }
                }
            }
        }
    }
}
