package com.visualworkbench.android

import android.os.Bundle
import android.app.Activity
import android.content.Intent
import android.Manifest
import android.content.pm.PackageManager
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.Lifecycle
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.MediaHandoff
import com.visualworkbench.android.editor.MediaIntents
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import com.visualworkbench.android.ui.WorkbenchApp
import com.visualworkbench.android.ui.WorkbenchTheme

class MainActivity : ComponentActivity() {
    internal lateinit var editor: EditorController
        private set
    private val handoff by lazy { MediaHandoff(this) }
    private var cameraToken: String? = null
    private var cameraStarting = false
    private val cameraPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) takePhoto()
        else editor.transferMessage("Camera permission was not granted. Choose an image from Photos or Files, or try the camera again.")
    }
    private val camera = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val token = cameraToken
        cameraToken = null
        if (token != null) editor.scope.launch {
            var lease: MediaHandoff.Lease? = null
            var delivered = false
            var retain = false
            try {
                val nonempty = withContext(Dispatchers.IO) {
                    lease = runCatching { handoff.resolve(token, camera = true) }.getOrNull()
                    (lease?.file?.length()?.let { it > 0 } == true).also { retain = it && result.resultCode == Activity.RESULT_OK }
                }
                val owned = lease
                if (owned != null && result.resultCode == Activity.RESULT_OK && nonempty) {
                    editor.cleanupTransfer { handoff.revoke(owned) }
                    editor.importCamera(owned)
                    delivered = true
                } else editor.transferMessage(if (result.resultCode == Activity.RESULT_OK) "The camera returned no full-resolution image." else "Camera capture canceled.")
            } finally {
                if (!delivered) editor.cleanupTransfer { lease?.let { if (retain) handoff.revoke(it) else handoff.remove(it, camera = true) } }
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        editor = ViewModelProvider(this)[EditorController::class.java]
        cameraToken = savedInstanceState?.getString("camera-token")
        val activeCamera = cameraToken
        editor.scope.launch { editor.cleanupTransfer { handoff.prune(System.currentTimeMillis(), activeCamera) } }
        if (savedInstanceState == null) acceptShare(intent)
        setContent { WorkbenchTheme(editor.preferences.darkTheme) { WorkbenchApp(editor, ::takePhoto) } }
    }

    override fun onNewIntent(intent: Intent) { super.onNewIntent(intent); setIntent(intent); acceptShare(intent) }
    override fun onSaveInstanceState(outState: Bundle) {
        outState.putString("camera-token", cameraToken)
        super.onSaveInstanceState(outState)
    }
    private fun acceptShare(intent: Intent) {
        try { MediaIntents.incoming(intent)?.let { editor.receiveImage(it) } }
        catch (_: Exception) { editor.transferMessage("This share does not contain a readable image. Try choosing it from Photos or Files.") }
    }
    private fun takePhoto() {
        if (cameraToken != null || cameraStarting) return
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            cameraPermission.launch(Manifest.permission.CAMERA); return
        }
        cameraStarting = true
        editor.scope.launch {
            var lease: MediaHandoff.Lease? = null
            var launched = false
            try {
                withContext(Dispatchers.IO) { lease = handoff.create(camera = true) }
                if (!lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) return@launch
                val owned = checkNotNull(lease)
                cameraToken = owned.token
                camera.launch(MediaIntents.camera(owned.uri))
                launched = true
            } catch (_: Exception) {
                editor.transferMessage("A full-resolution camera app is unavailable. Choose an existing image instead.")
            } finally {
                cameraStarting = false
                if (!launched) {
                    cameraToken = null
                    editor.cleanupTransfer { lease?.let { handoff.remove(it, camera = true) } }
                }
            }
        }
    }

    override fun onPause() { editor.onBackground(); super.onPause() }
}
