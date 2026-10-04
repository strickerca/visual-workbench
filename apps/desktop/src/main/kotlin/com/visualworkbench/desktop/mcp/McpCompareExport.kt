package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.TransferFiles
import com.visualworkbench.desktop.publishExport
import com.visualworkbench.shared.McpInboxReceipt
import com.visualworkbench.shared.McpInboxSide
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import java.nio.file.Path

/** Exact immutable bytes, explicit owner-selected path, atomic no-clobber
 * publication through the existing reviewed transfer implementation. */
internal suspend fun exportMcpOriginal(owner:DesktopMcpCoordinator,receipt:McpInboxReceipt,side:McpInboxSide,destination:Path,privateWork:Path) {
    require(destination.fileName.toString().endsWith(".png",ignoreCase=true))
    val bytes=owner.original(receipt,side)
    require(bytes.size in 1..4*1024*1024)
    val workspace=TransferFiles(privateWork).create()
    try {
        val source=workspace.stage(bytes)
        publishExport(source,destination,bytes.size.toULong()){true}
    } finally {withContext(NonCancellable){workspace.close()}}
}
