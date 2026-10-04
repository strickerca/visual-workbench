package com.visualworkbench.android.instructions

import android.graphics.Bitmap
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.*
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import org.junit.Assert.*
import org.junit.Test

/** Real native workflow/controller persistence using only an owned synthetic
 * project. No recognizer, microphone, personal image or network is accessed. */
class InstructionRoundTripInstrumentedTest {
    @Test fun markerInstructionRoleEntryUndoAndReopenRetainCanonicalIdentity() = runBlocking {
        val cache = InstrumentationRegistry.getInstrumentation().targetContext.cacheDir.canonicalFile
        val root = File(cache, "instruction-roundtrip-${UUID.randomUUID()}").canonicalFile
        check(root.parentFile == cache && root.mkdir())
        val core = workbenchCore(); val now = System.currentTimeMillis()
        val projectId = core.newId(now.toULong()); val documentId = core.newId(now.toULong()); val layerId = core.newId(now.toULong()); val deviceId = core.newDeviceId()
        val source = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888).let { bitmap ->
            try { bitmap.eraseColor(-1); ByteArrayOutputStream().also { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }.toByteArray() }
            finally { bitmap.recycle() }
        }
        val path = File(root, "project").absolutePath
        var project: WorkbenchProject? = null
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        var editor: InstructionEditor? = null
        try {
            val p = core.create(CreateProject(path, projectId, documentId, layerId, deviceId, "Owned instruction fixture", source, now)); project = p
            var attachment = InstructionAttachment(p, p.document(documentId))
            val controller = InstructionEditor(scope, Mutex(), { attachment }, { core.newId(System.currentTimeMillis().toULong()) },
                { active -> val info = active.info(); WorkflowMetadata(core.newId(System.currentTimeMillis().toULong()), deviceId, info.nextLamport, System.currentTimeMillis()) },
                { attachment = InstructionAttachment(p, p.document(documentId)) }, InstructionEntryMethod.PhoneKeyboard)
            editor = controller
            suspend fun action(block: () -> Unit) { withContext(Dispatchers.Main.immediate) { block() }; withTimeout(15_000) { while (controller.state.value.busy) delay(10) } }
            action { controller.place(Point(10.0, 10.0), ObjectStyle(0x203040ffu, 2.0)) }
            val first = controller.state.value.field!!
            action { controller.method(InstructionEntryMethod.Voice); controller.update("Preserve \"this\" entire literal instruction."); controller.role(InstructionRole.Preserve) }
            action { controller.save() }
            action { controller.place(Point(40.0, 40.0), ObjectStyle(0x805020ffu, 3.0)) }
            val second = controller.state.value.field!!
            val before = instructionWorkflows(p).document(documentId)
            assertEquals(listOf(1u, 2u), before.markers.map { it.number })
            assertEquals(InstructionRole.Preserve, before.instructions.single { it.instructionId == first.instructionId }.role)
            assertEquals(InstructionEntryMethod.Voice, before.instructions.single { it.instructionId == first.instructionId }.entryMethod)
            action { controller.deleteMarker(first.targetIds.single()) }
            val deleted = instructionWorkflows(p).document(documentId)
            assertEquals(second.instructionId, deleted.markers.single().instructionId)
            assertEquals(1u, deleted.markers.single().number)
            assertFalse(deleted.instructions.any { it.instructionId == first.instructionId })
            val info = p.info()
            p.undoRedo(EditOptions(core.newId(System.currentTimeMillis().toULong()), documentId, deviceId, info.nextLamport, System.currentTimeMillis()), false)
            val restored = instructionWorkflows(p).document(documentId)
            assertEquals(before.markers.map { it.objectId }, restored.markers.map { it.objectId })
            assertEquals(before.instructions.map { it.text }, restored.instructions.map { it.text })
            withContext(Dispatchers.Main.immediate) { controller.detach() }; editor = null
            p.close(); project = null
            val reopened = core.open(path); project = reopened
            assertEquals(restored, instructionWorkflows(reopened).document(documentId))
        } finally {
            withContext(NonCancellable) {
                withContext(Dispatchers.Main.immediate) { editor?.detach() }
                project?.close(); scope.cancel()
                check(root.canonicalFile.parentFile == cache && root.name.startsWith("instruction-roundtrip-"))
                check(root.deleteRecursively())
            }
        }
    }
}
