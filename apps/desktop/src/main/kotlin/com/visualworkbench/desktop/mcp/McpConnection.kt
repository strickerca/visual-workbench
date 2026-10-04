package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.flow.StateFlow
import java.nio.file.Path

/** Owner-only control channel. No implementation receives a bearer token. */
internal interface McpConnection {
    val agents:StateFlow<List<McpAgent>>
    val port:Int
    suspend fun publish(directory:Path,manifestSha256:String)
    suspend fun unpublish(packageId:String,target:String,manifestSha256:String)
    suspend fun previewClaude(packageId:String,manifestSha256:String):McpClaudePreview
    suspend fun pushClaude(connection:String,displayed:McpClaudePreview)
    suspend fun grant(connection:String,selectors:List<String>,lifetimeMs:Int)
    suspend fun revoke(connection:String)
    suspend fun close()
}
internal class NativeMcpConnection(private val host:DesktopMcpHost):McpConnection {
    override val agents get()=host.agents
    override val port get()=host.port
    override suspend fun publish(directory:Path,manifestSha256:String){host.publish(directory,manifestSha256)}
    override suspend fun unpublish(packageId:String,target:String,manifestSha256:String)=host.unpublish(packageId,target,manifestSha256)
    override suspend fun previewClaude(packageId:String,manifestSha256:String)=host.previewClaude(packageId,manifestSha256)
    override suspend fun pushClaude(connection:String,displayed:McpClaudePreview){
        val receipt=host.pushClaude(connection,displayed)
        // This acknowledges the transport write only, never agent delivery or use.
        require(receipt["written_to_transport"]==true && receipt["agent_delivery_confirmed"]==false)
    }
    override suspend fun grant(connection:String,selectors:List<String>,lifetimeMs:Int)=host.grantCapture(connection,selectors,lifetimeMs)
    override suspend fun revoke(connection:String)=host.revokeCapture(connection)
    override suspend fun close()=host.shutdown()
}
