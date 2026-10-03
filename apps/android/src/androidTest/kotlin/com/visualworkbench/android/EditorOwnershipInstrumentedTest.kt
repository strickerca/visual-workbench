package com.visualworkbench.android

import android.app.Application
import android.graphics.Bitmap
import android.os.SystemClock
import android.util.Log
import androidx.lifecycle.ViewModelStore
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.EditorController
import com.visualworkbench.android.editor.MediaHandoff
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

/** A real native project is held at an injected loader gate while final disposal
 * begins. This tests close ordering without timing a fast storage call. */
@RunWith(AndroidJUnit4::class)
class EditorOwnershipInstrumentedTest {
    @Test fun failedCameraImportRetainsTheOnlyOriginalAcrossFinalDisposal() = runBlocking {
        val app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual = workbenchCore(); val handoff = MediaHandoff(app)
        val lease = handoff.create(true); val original = "synthetic captured original".toByteArray()
        lease.file.writeBytes(original)
        val attempted = CompletableDeferred<Unit>()
        val refusing = object : WorkbenchCore by actual, WorkbenchStreamingCore {
            override suspend fun createFile(options: CreateFileProject): WorkbenchProject {
                assertArrayEquals(original, File(options.sourcePath).readBytes())
                attempted.complete(Unit)
                throw CoreFailure(CoreFailureKind.Unsupported)
            }
        }
        val store = ViewModelStore(); var controller: EditorController? = null
        try {
            withContext(Dispatchers.Main) {
                controller = EditorController(app, refusing); store.put("retained-camera", checkNotNull(controller))
                checkNotNull(controller).importCamera(lease)
            }
            withTimeout(15_000) { attempted.await() }
            withContext(Dispatchers.Main) { store.clear() }
            withTimeout(15_000) { checkNotNull(controller?.scope?.coroutineContext?.get(Job)).join() }
            assertArrayEquals(original, lease.file.readBytes())
            assertTrue(handoff.captures().any { it.token == lease.token })
            assertTrue(lease.file.setLastModified(1_000)); handoff.prune(System.currentTimeMillis())
            assertArrayEquals(original, lease.file.readBytes())
        } finally {
            withContext(NonCancellable + Dispatchers.Main) { store.clear() }
            controller?.scope?.coroutineContext?.get(Job)?.let { withContext(NonCancellable) { withTimeout(15_000) { it.join() } } }
            handoff.remove(lease, true)
        }
    }

    @Test fun queuedCameraImportKeepsItsLeaseUntilCancelledNativeProducerSettles() = runBlocking {
        val app = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual = workbenchCore()
        val streaming = actual as WorkbenchStreamingCore
        val handoff = MediaHandoff(app)
        val lease = handoff.create(camera = true)
        val bitmap = Bitmap.createBitmap(37, 29, Bitmap.Config.ARGB_8888)
        try { bitmap.eraseColor(0xff234567.toInt()); lease.file.outputStream().use { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) } }
        finally { bitmap.recycle() }
        val acquired = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>(); val cleaned = CompletableDeferred<Unit>()
        val nativeSettled = AtomicBoolean(false)
        var path: String? = null
        val gated = object : WorkbenchCore by actual, WorkbenchStreamingCore {
            override suspend fun createFile(options: CreateFileProject): WorkbenchProject {
                val handle = streaming.createFile(options)
                path = options.path; acquired.complete(Unit)
                withContext(NonCancellable) { release.await() }
                nativeSettled.set(true)
                return handle
            }
        }
        val store = ViewModelStore(); var controller: EditorController? = null
        var closed = false
        try {
            withContext(Dispatchers.Main) {
                controller = EditorController(app, gated)
                store.put("camera-ownership", checkNotNull(controller))
                // Intentionally arrives while initialization is still busy.
                checkNotNull(controller).receiveImage(lease.uri) {
                    check(nativeSettled.get()) { "Camera lease retired before its native reader settled" }
                    handoff.remove(lease, camera = true); cleaned.complete(Unit)
                }
            }
            withTimeout(45_000) {
                var progress = SystemClock.uptimeMillis() + 5_000
                while (!acquired.isCompleted) {
                    if (SystemClock.uptimeMillis() >= progress) { Log.i("VW_EDITOR_TEST", "phase=camera-import awaiting-native=true"); progress += 5_000 }
                    delay(25)
                }
            }
            withContext(Dispatchers.Main) { checkNotNull(controller).cancelTransfer(); store.clear() }
            delay(50)
            assertFalse(cleaned.isCompleted); assertTrue(lease.file.exists())
            release.complete(Unit)
            withTimeout(15_000) { cleaned.await(); checkNotNull(controller?.scope?.coroutineContext?.get(Job)).join() }
            assertFalse(lease.file.exists())
            val reopened = actual.open(checkNotNull(path))
            try { assertEquals(1, reopened.info().documentIds.size) } finally { reopened.close() }
            closed = true
        } finally {
            release.complete(Unit)
            withContext(NonCancellable + Dispatchers.Main) { store.clear() }
            controller?.scope?.coroutineContext?.get(Job)?.let { withContext(NonCancellable) { withTimeout(15_000) { it.join() } } }
            if (lease.file.exists()) handoff.remove(lease, camera = true)
            if (closed) path?.let {
                val owned = File(it).canonicalFile
                assertEquals(File(app.filesDir, "projects").canonicalFile, owned.parentFile)
                assertTrue(owned.deleteRecursively())
            }
        }
    }

    @Test fun finalDisposalClosesAProjectReturnedByAnAlreadyRunningLoader() = runBlocking {
        val app=InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val actual=workbenchCore();val now=System.currentTimeMillis();val id=actual.newId(now.toULong())
        val root=File(app.filesDir,"projects").canonicalFile
        assertTrue(root.isDirectory||root.mkdirs())
        val path=File(root,id).canonicalFile;assertEquals(root,path.parentFile)
        val bitmap=Bitmap.createBitmap(64,64,Bitmap.Config.ARGB_8888)
        val bytes=try{bitmap.eraseColor(android.graphics.Color.WHITE);ByteArrayOutputStream().also{assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG,100,it))}.toByteArray()}finally{bitmap.recycle()}
        val initial=actual.create(CreateProject(path.absolutePath,id,actual.newId(now.toULong()),actual.newId(now.toULong()),actual.newDeviceId(),"Owned shutdown fixture",bytes,now))
        initial.close()
        val acquired=CompletableDeferred<Unit>();val release=CompletableDeferred<Unit>();val armed=AtomicBoolean(false);val targetPath=path.absolutePath
        val gated=object:WorkbenchCore by actual {
            override suspend fun open(path:String):WorkbenchProject {
                val handle=actual.open(path)
                if(path==targetPath && armed.compareAndSet(true,false)) {
                    acquired.complete(Unit)
                    try{release.await()}catch(error:Throwable){withContext(NonCancellable){handle.close()};throw error}
                }
                return handle
            }
        }
        val store=ViewModelStore();var controller:EditorController?=null;var closed=false
        try {
            withContext(Dispatchers.Main){controller=EditorController(app,gated);store.put("owned-editor",checkNotNull(controller))}
            val editor=checkNotNull(controller)
            withTimeout(45_000){var progress=SystemClock.uptimeMillis()+5_000;while(withContext(Dispatchers.Main){editor.busy}){
                if(SystemClock.uptimeMillis()>=progress){Log.i("VW_EDITOR_TEST","phase=ownership-startup waiting=true");progress+=5_000};delay(25)
            }}
            armed.set(true)
            withContext(Dispatchers.Main){editor.open(id)}
            withTimeout(15_000){acquired.await()}
            withContext(Dispatchers.Main){store.clear()}
            assertFalse(checkNotNull(editor.scope.coroutineContext[Job]).isCompleted)
            release.complete(Unit)
            withTimeout(15_000){checkNotNull(editor.scope.coroutineContext[Job]).join()}
            // A retained, cleared ViewModel must not keep the SQLite lock alive.
            val reopened=actual.open(path.absolutePath)
            try{assertEquals(id,reopened.info().projectId)}finally{reopened.close()}
            closed=true
        } finally {
            release.complete(Unit)
            withContext(NonCancellable+Dispatchers.Main){store.clear()}
            controller?.scope?.coroutineContext?.get(Job)?.let{withContext(NonCancellable){withTimeout(15_000){it.join()}}}
            if(closed){assertEquals(root,path.canonicalFile.parentFile);assertTrue(path.deleteRecursively())}
            else Log.e("VW_EDITOR_TEST","owned_project_cleanup=false reason=close_not_confirmed")
        }
    }
}
