package com.visualworkbench.android

import android.app.Application
import android.content.Context
import android.content.SharedPreferences
import java.io.File
import java.nio.file.Files
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.android.editor.ProjectRepository
import com.visualworkbench.shared.workbenchCore
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class EditorFixtureIdentityInstrumentedTest {
    @Test fun isolatedEditorsNeverReadOrWriteTheInstalledAppsIdentityOrCounters() = runBlocking<Unit> {
        val base = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext as Application
        val ordinary = base.getSharedPreferences("local-identity", Context.MODE_PRIVATE)
        val original = ordinary.all.toMap()
        val first = IsolatedEditorApplication(base)
        val second = IsolatedEditorApplication(base)
        try {
            val firstPrefs = first.getSharedPreferences("local-identity", Context.MODE_PRIVATE)
            val secondPrefs = second.getSharedPreferences("local-identity", Context.MODE_PRIVATE)
            assertNull(firstPrefs.getString("device", null)); assertNull(secondPrefs.getString("device", null))
            val core = workbenchCore()
            val firstId = core.newDeviceId(); val secondId = core.newDeviceId()
            assertTrue(firstPrefs.edit().putString("device", firstId).putLong("lamport-reserved", 17).commit())
            assertTrue(secondPrefs.edit().putString("device", secondId).putLong("lamport-reserved", 29).commit())
            val a = ProjectRepository(first, core); val b = ProjectRepository(second, core)
            a.initialize(); b.initialize()
            assertEquals(firstId, a.deviceId); assertEquals(secondId, b.deviceId)
            assertEquals(17uL, a.lamport()); assertEquals(29uL, b.lamport())
            assertTrue("Synthetic fixtures changed the installed app identity", original == ordinary.all)
            assertNotEquals(first.filesDir, second.filesDir)
            assertNotEquals(base.filesDir.canonicalFile, first.filesDir.canonicalFile)
        } finally {
            withContext(NonCancellable + Dispatchers.IO) { try { first.close() } finally { second.close() } }
        }
        assertTrue("Fixture cleanup changed the installed app identity", original == ordinary.all)
    }
}

/** Application-only storage namespace for synthetic EditorController fixtures.
 * It keeps identity, settings, projects and private caches out of the installed
 * app's namespaces. It is not an Activity/FileProvider or real-session fixture.
 * Call close only after ViewModelStore.clear AND its actual scope Job.join. */
internal class IsolatedEditorApplication(private val base: Application) : Application(), AutoCloseable {
    private val parent = base.cacheDir.canonicalFile
    private val owned = Files.createTempDirectory(parent.toPath(), "editor-fixture-").toFile().canonicalFile
    private val prefix = owned.name + "-"
    private val preferences = linkedSetOf<String>()
    private var closed = false
    private val files = directory("files")
    private val cache = directory("cache")
    private val noBackup = directory("no-backup")
    init { attachBaseContext(base) }

    private fun directory(name: String): File = File(owned, name).also { check(it.mkdir()) }
    override fun getApplicationContext(): Context = this
    override fun getFilesDir(): File = files
    override fun getCacheDir(): File = cache
    override fun getNoBackupFilesDir(): File = noBackup
    @Synchronized override fun getSharedPreferences(name: String, mode: Int): SharedPreferences {
        check(!closed)
        require(name in setOf("local-identity", "editor-preferences"))
        val isolated = prefix + name
        preferences += isolated
        return base.getSharedPreferences(isolated, mode)
    }
    @Synchronized override fun close() {
        if (closed) return
        // Walk only this exclusively created directory; refuse redirected paths
        // before deleting any child. Joined native close precedes this method.
        check(owned.canonicalFile == owned.absoluteFile && owned.parentFile == parent)
        val entries = owned.walkTopDown().take(2049).toList()
        check(entries.size <= 2048)
        check(entries.all { it.canonicalFile == it.absoluteFile && it.toPath().startsWith(owned.toPath()) })
        for (name in preferences) {
            check(name.startsWith(prefix))
            check(base.getSharedPreferences(name, Context.MODE_PRIVATE).edit().clear().commit())
            check(base.deleteSharedPreferences(name))
        }
        for (file in entries.asReversed()) check(file.delete())
        closed = true
    }
}
