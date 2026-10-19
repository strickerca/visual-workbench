@file:OptIn(ExperimentalUnsignedTypes::class)

package com.visualworkbench.android.editor

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import org.junit.Assert.*
import org.junit.Test

private class CallerProject(private val closed: () -> Unit = {}) : WorkbenchProject {
    override val changes: Flow<ProjectChange> = emptyFlow()
    override suspend fun info(): ProjectInfo = error("unused")
    override suspend fun document(documentId: String): DocumentSnapshot = error("unused")
    override suspend fun background(documentId: String, memoryBudgetBytes: ULong, assumeUntaggedSrgb: Boolean): BackgroundImage = error("unused")
    override suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke = error("unused")
    override suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo = error("unused")
    override suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo = error("unused")
    override suspend fun render(documentId: String, rectangle: Rect?): RenderList = error("unused")
    override suspend fun export(options: ExportOptions): ExportResult = error("unused")
    override suspend fun close() = closed()
}

class AndroidPlatformCallersTest {
    private val binding = WorkflowBinding("project", "document", 7uL, "revision")
    private val info = ProjectInfo("project", "Canvas", "device", 1uL, false, false, 7uL, "revision", listOf("document"))
    private val document = DocumentSnapshot("document", "Canvas", 80u, 40u, 8u, emptyList(), RenderList(info, emptyList()))
    private val camera = Camera(Point(40.0, 20.0), 1.0, 0.0, 80.0, 40.0)

    @Test fun capturedSourceIntentRejectsReplayedOutputAndChangedPickerState() {
        val project = CallerProject()
        val selection = ExportSelection(viewOnly = true)
        val source = ExportOptions("document", region = Rect(10.0, 5.0, 20.0, 10.0))
        val projected = source.copy(region = Rect(20.0, 10.0, 40.0, 20.0))
        val geometry = ImageOutputExportGeometry(binding, "a".repeat(64), 80u, 40u, 160u, 80u,
            SelectionRegion(20u, 10u, 40u, 20u), 40u, 20u)
        val intent = ExportCallerIntent(project, 3L, null, camera, selection, binding, source, projected, geometry)
        assertTrue(intent.matches(project, 3L, document, info, camera, selection))
        assertFalse(intent.matches(project, 3L, document, info, camera.copy(scale = 2.0), selection))
        assertFalse(intent.matches(project, 4L, document, info, camera, selection))
        assertFalse(intent.matches(project, 3L, document, info.copy(stateHash = "other"), camera, selection))
        assertFalse(intent.matches(project, 3L, document, info, camera, selection.copy(marked = false)))
        assertFalse(ExportCallerIntent(project, 3L, null, camera, selection, binding, source, source, geometry)
            .matches(project, 3L, document, info, camera, selection))
    }

    @Test fun giantOwnersCloseInReverseOrderAfterSuccessfulDelivery() = runBlocking {
        val order = mutableListOf<String>()
        val project = CallerProject { order += "project" }
        val picked = picked(order, project)
        val result = publishAndroidGiantPicked(picked, GiantCacheBuildOptions(1uL, 1uL, true),
            { GiantProjectOptions("/private/destination", "project", "document", "layer", "device", "Large JPEG", 1L, 1uL) }, {})
        assertSame(project, result)
        assertEquals(listOf("prepare", "describe", "build", "publish", "built-close", "prepared-close", "picked-close"), order)
    }

    @Test fun cancellationAfterPublishClosesUndeliveredProjectAndAllImportOwners() = runBlocking {
        val order = mutableListOf<String>()
        val project = CallerProject { order += "project-close" }
        val picked = picked(order, project, cancelAfterPublish = true)
        val job = launch(start = CoroutineStart.UNDISPATCHED) {
            publishAndroidGiantPicked(picked, GiantCacheBuildOptions(1uL, 1uL, true),
                { GiantProjectOptions("/private/destination", "project", "document", "layer", "device", "Large JPEG", 1L, 1uL) }, {})
            fail("cancelled project was delivered")
        }
        job.join()
        assertTrue(job.isCancelled)
        assertEquals(listOf("prepare", "describe", "build", "publish", "project-close", "built-close", "prepared-close", "picked-close"), order)
    }

    @Test fun nonJpegOriginalNeverReachesTileBuildOrPublication() = runBlocking {
        val order = mutableListOf<String>()
        try {
            publishAndroidGiantPicked(picked(order, CallerProject(), format = "png"), GiantCacheBuildOptions(1uL, 1uL, true),
                { error("destination was used") }, {})
            fail("non-JPEG admitted")
        } catch (_: IllegalArgumentException) { }
        assertEquals(listOf("prepare", "describe", "prepared-close", "picked-close"), order)
    }

    private fun picked(order: MutableList<String>, project: WorkbenchProject,
        format: String = "jpeg", cancelAfterPublish: Boolean = false): GiantPickedOriginal = object : GiantPickedOriginal {
        override suspend fun prepare(): GiantPreparedOriginal {
            order += "prepare"
            return object : GiantPreparedOriginal {
                override suspend fun describe(): GiantOriginalInfo {
                    order += "describe"
                    return GiantOriginalInfo("a".repeat(64), "b".repeat(64), 1uL, 80u, 40u, 80u, 40u, 1u, format)
                }
                override suspend fun build(options: GiantCacheBuildOptions): GiantBuiltImage {
                    order += "build"
                    return object : GiantBuiltImage {
                        override suspend fun publish(options: GiantProjectOptions): WorkbenchProject {
                            order += "publish"
                            if (cancelAfterPublish) currentCoroutineContext().cancel()
                            return project
                        }
                        override suspend fun rebuild(project: WorkbenchProject, binding: WorkflowBinding) = error("unused")
                        override suspend fun close() { order += "built-close" }
                    }
                }
                override suspend fun close() { order += "prepared-close" }
            }
        }
        override suspend fun close() { order += "picked-close" }
    }
}
