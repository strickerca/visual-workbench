package com.visualworkbench.desktop

import com.visualworkbench.shared.GiantInputInfo
import com.visualworkbench.shared.GiantPreparedSource
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Quarantine this owner on a failed close. Later layers may still be used by
 * an uncertain producer, so no finally clause may delete their private files. */
internal class GiantImportResources(val workspace:TransferWorkspace) {
    var source:GiantPreparedSource?=null
    var producer:GiantProducer?=null
    suspend fun close()=withContext(NonCancellable){
        settleGiantResources(
            {producer?.close();producer=null},
            {source?.close();source=null},
            {workspace.close()},
        )
    }
}
internal suspend fun settleGiantResources(producer:suspend()->Unit,source:suspend()->Unit,files:suspend()->Unit){
    producer();source();files()
}
/** Two immutable-original copies plus producer/publication pyramid scratch.
 * Checked before starting libvips; external selected file is never mutated. */
internal fun admitGiantWorkingDisk(info:GiantInputInfo):ULong {
    val pixels=info.width.toULong()*info.height.toULong()
    if(pixels==0uL||pixels>1_000_000_000uL||info.encodedBytes==0uL||info.encodedBytes>4uL*1024uL*1024uL*1024uL)
        throw HandoffFailure("The giant source exceeds the admitted original or pixel bound.")
    val required=info.encodedBytes*2uL+pixels*12uL+64uL*1024uL*1024uL
    if(required>16uL*1024uL*1024uL*1024uL)throw HandoffFailure("The giant producer and immutable publication would exceed the 16 GiB working-disk budget.")
    return required
}

/** Publication may have completed before cancellation of foreign delivery. The
 * caller can inspect this exact new project path; no retry overwrites it. */
internal class GiantImportCancelled(val destination:java.nio.file.Path):kotlinx.coroutines.CancellationException("Giant import cancelled")
