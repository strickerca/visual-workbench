package com.visualworkbench.shared

import com.visualworkbench.bindings.core.GiantImageException
import com.visualworkbench.bindings.core.GiantPickedOriginal as NPicked
import com.visualworkbench.bindings.core.PreparedOriginal as NPrepared
import com.visualworkbench.bindings.core.GiantBuiltCache as NBuilt
import com.visualworkbench.bindings.core.GiantOriginalOptions as NOriginalOptions
import com.visualworkbench.bindings.core.GiantCacheBuildOptions as NBuildOptions
import com.visualworkbench.bindings.core.GiantProjectOptions as NProjectOptions
import com.visualworkbench.bindings.core.WorkflowBinding as NBinding
import com.visualworkbench.bindings.core.openGiantOriginal as openNativeOriginal
import com.visualworkbench.bindings.core.ProjectSession
import com.visualworkbench.bindings.core.CoreException

private val importRetirements = GiantImportRetirements()
private suspend fun <T> importErrors(block: suspend () -> T): T = try { block() }
    catch (error: GiantImageException) { throw mapGiantFailure(error) }

private fun ownPicked(value: NPicked): GiantImportOwner<NPicked> = GiantImportOwner(value,
    { importErrors { it.closeSession() } }, { it.destroy() }, importRetirements)
private fun ownPrepared(value: NPrepared): GiantImportOwner<NPrepared> = GiantImportOwner(value,
    { importErrors { it.closeSession() } }, { it.destroy() }, importRetirements)
private fun ownBuilt(value: NBuilt): GiantImportOwner<NBuilt> = GiantImportOwner(value,
    { importErrors { it.closeSession() } }, { it.destroy() }, importRetirements)
private fun ownProject(value: ProjectSession): GiantImportOwner<ProjectSession> = GiantImportOwner(value,
    { drainUnadoptedGiants(); try { it.closeSession() } catch (error: CoreException) { throw mapFailure(error) } },
    { it.destroy() }, importRetirements)

internal suspend fun openGiantOriginalNative(options: GiantOriginalOptions): GiantPickedOriginal {
    importRetirements.drain()
    val native = NOriginalOptions(options.sourcePath, options.workDirectory, options.maxOriginalBytes,
        options.maxWorkDiskBytes, options.assumeUntaggedSrgb)
    return importErrors { acquireGiantImport({ openNativeOriginal(native, it) }, ::ownPicked, ::NativePicked) }
}
private class NativePicked(private val owner: GiantImportOwner<NPicked>) : GiantPickedOriginal {
    override suspend fun prepare(): GiantPreparedOriginal = owner.call { handle ->
        importErrors { acquireGiantImport({ handle.prepare(it) }, ::ownPrepared, ::NativePrepared) }
    }
    override suspend fun close(): Unit = owner.close()
}
private class NativePrepared(private val owner: GiantImportOwner<NPrepared>) : GiantPreparedOriginal {
    override suspend fun describe(): GiantOriginalInfo = owner.call {
        val value = it.describeOriginal()
        GiantOriginalInfo(value.assetId, value.sha256, value.encodedBytes, value.rawWidth,
            value.rawHeight, value.width, value.height, value.orientation, value.format)
    }
    override suspend fun build(options: GiantCacheBuildOptions): GiantBuiltImage = owner.call { handle ->
        val native = NBuildOptions(options.diskCacheBytes, options.timeoutMs, options.assumeUntaggedSrgb)
        importErrors { acquireGiantImport({ handle.buildCache(native, it) }, ::ownBuilt, ::NativeBuilt) }
    }
    override suspend fun close(): Unit = owner.close()
}
private class NativeBuilt(private val owner: GiantImportOwner<NBuilt>) : GiantBuiltImage {
    override suspend fun publish(options: GiantProjectOptions): WorkbenchProject = owner.call { handle ->
        val native = NProjectOptions(options.path, options.projectId, options.documentId, options.layerId,
            options.deviceId, options.title, options.nowMs, options.diskCacheBytes)
        importErrors { acquireGiantImport({ handle.publishProject(native, it) }, ::ownProject, ::wrapNativeGiantProject) }
    }
    override suspend fun rebuild(project: WorkbenchProject, binding: WorkflowBinding): Unit = owner.call { built ->
        val nativeProject = try { nativeProjectHandle(project) }
            catch (_: SessionFailure) { throw GiantImageFailure(GiantImageFailureKind.Invalid) }
        importErrors { settledStream { nativeProject.rebuildGiantImage(built,
            NBinding(binding.projectId, binding.documentId, binding.hostSeq, binding.stateHash), it) } }
    }
    override suspend fun close(): Unit = owner.close()
}
