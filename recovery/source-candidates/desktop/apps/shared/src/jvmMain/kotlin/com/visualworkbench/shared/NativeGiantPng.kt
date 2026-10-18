package com.visualworkbench.shared

import com.visualworkbench.bindings.core.GiantImage as NImage
import com.visualworkbench.bindings.core.GiantPngExport as NOptions
import com.visualworkbench.bindings.core.QueryRect

/** Existing streaming ownership joins the actual native producer on cancellation.
 * A fully published output can survive a lost/cancelled result; never delete it
 * merely because the receipt was not delivered. Destination must be absent. */
internal suspend fun nativeGiantPng(handle:NImage,options:GiantPngOptions):GiantPngReceipt = settledStream { cancel ->
    val result=handle.exportPng(NOptions(workflowBinding(options.binding),options.outputPath,
        options.marked,options.region?.let{QueryRect(it.x,it.y,it.width,it.height)},
        options.assumeUntaggedSrgb,options.memoryBudgetBytes,options.maxEncodedBytes),cancel)
    GiantPngReceipt(workflowInfo(result.revision),result.encodedBytes,result.blake3,
        result.metadataJson,result.estimatedPeakBytes)
}
