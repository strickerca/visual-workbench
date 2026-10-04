package com.visualworkbench.desktop.mcp

import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.security.MessageDigest

/** Pins come from the signed/generated application resource, never the agent or
 * the runtime directory itself. Every runtime file (including node_modules and
 * native helpers) must be enumerated. Central packaging supplies this inventory. */
internal class McpRuntime private constructor(val root: Path) {
    val executable: Path get() = root.resolve("vw-mcp-host.exe")
    companion object {
        fun verify(root: Path, trustedHashes: Map<String,String>,serverOnly:Boolean=false): McpRuntime {
            require(trustedHashes.size in 4..20_000)
            val base=root.toAbsolutePath().normalize()
            require(base==root && Files.isDirectory(base,LinkOption.NOFOLLOW_LINKS))
            noRedirect(base)
            var total=0L
            trustedHashes.forEach { (name, expected) ->
                require(name.length in 1..256 && !name.contains('\\') && name.split('/').all { it.isNotEmpty() && it !in listOf(".","..") })
                require(expected.matches(Regex("[0-9a-f]{64}")))
                val path=base.resolve(name).normalize();require(path.startsWith(base));noRedirect(path)
                require(Files.isRegularFile(path,LinkOption.NOFOLLOW_LINKS))
                val size=Files.size(path);require(size in 0..256L*1024*1024);total=Math.addExact(total,size);require(total<=1024L*1024*1024)
                val digest=MessageDigest.getInstance("SHA-256")
                Files.newInputStream(path).use { input -> val buffer=ByteArray(65536);var read=0L;while(true){val n=input.read(buffer);if(n<0)break;read+=n;require(read<=size);digest.update(buffer,0,n)};require(read==size) }
                require(digest.digest().joinToString(""){"%02x".format(it)}==expected)
            }
            val required=if(serverOnly)listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs") else listOf("VisualWorkbench.exe","vw-mcp-host.exe","vw-mcp.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs")
            for(name in required+listOf("vw-codex-host.exe", "mcp/codex/main.mjs", "mcp/codex/client.mjs", "mcp/codex/package.mjs", "mcp/codex/process.mjs", "mcp/codex/schema.mjs", "mcp/codex/schema-worker.mjs", "mcp/codex/image-profiles.json")) require(trustedHashes.containsKey(name))
            // Node searches dependencies on disk: an unlisted injection cannot be
            // admitted alongside valid pins. This directory is a dedicated bundle.
            Files.walk(base,32).use { paths -> var count=0;paths.forEach { p -> require(++count<=30_000);noRedirect(p);if(Files.isRegularFile(p,LinkOption.NOFOLLOW_LINKS)){require(trustedHashes.containsKey(base.relativize(p).toString().replace('\\','/')))}else require(p.nameCount-base.nameCount<32) } }
            return McpRuntime(base)
        }
        private fun noRedirect(path:Path) {
            var current=path.root ?: error("Absolute runtime required")
            for(part in path){current=current.resolve(part);require(!Files.isSymbolicLink(current));require(current.toRealPath()==current.toAbsolutePath().normalize())}
        }
    }
}
