package com.visualworkbench.desktop.mcp

import java.io.IOException
import java.nio.file.FileVisitResult
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.SimpleFileVisitor
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest

/** Pins come from the signed/generated application resource, never the agent or
 * the runtime directory itself. Every runtime file (including node_modules and
 * native helpers) must be enumerated. Central packaging supplies this inventory. */
internal class McpRuntime private constructor(val root: Path) {
    val executable: Path get() = root.resolve("vw-mcp-host.exe")
    companion object {
        internal const val MAX_DEPTH=32
        private const val MAX_ENTRIES=30_000
        private const val READ_BUFFER=1 shl 20
        /** Every start re-hashes every admitted byte. Redirection is refused by
         * one canonical check of the root plus one tree walk whose directory
         * entries carry the names exactly as stored and their own attributes:
         * a reparse point, symlink, special file, 8.3 alias, case variant or
         * unlisted file anywhere below the root is refused before and again
         * after hashing. This replaces a per-file toRealPath of every ancestor
         * (O(files x depth^2) directory lookups, about 60 s for the packaged
         * 1,514-file bundle) with the same refusals at O(entries) cost. */
        fun verify(root: Path, trustedHashes: Map<String,String>,serverOnly:Boolean=false,progress:McpStartupProgress?=null): McpRuntime {
            require(trustedHashes.size in 4..20_000)
            val base=root.toAbsolutePath().normalize()
            require(base==root && Files.isDirectory(base,LinkOption.NOFOLLOW_LINKS))
            noRedirect(base)
            trustedHashes.forEach { (name, expected) ->
                require(name.length in 1..256 && !name.contains('\\') && name.split('/').let{parts->parts.size<=MAX_DEPTH && parts.all { it.isNotEmpty() && it !in listOf(".","..") }})
                require(expected.matches(Regex("[0-9a-f]{64}")))
                require(base.resolve(name).normalize().startsWith(base))
            }
            val required=if(serverOnly)listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs") else listOf("VisualWorkbench.exe","vw-mcp-host.exe","vw-mcp.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs")
            for(name in required+listOf("vw-codex-host.exe", "mcp/codex/main.mjs", "mcp/codex/client.mjs", "mcp/codex/package.mjs", "mcp/codex/process.mjs", "mcp/codex/schema.mjs", "mcp/codex/schema-worker.mjs", "mcp/codex/image-profiles.json")) require(trustedHashes.containsKey(name))
            // Node searches dependencies on disk: an unlisted injection cannot be
            // admitted alongside valid pins. This directory is a dedicated bundle.
            // Exact (case-sensitive) equality with stored names also proves each
            // manifest path names that stored entry rather than an alias.
            require(storedFiles(base)==trustedHashes.keys)
            var total=0L
            val buffer=ByteArray(READ_BUFFER)
            trustedHashes.forEach { (name, expected) ->
                val path=base.resolve(name)
                val size=Files.size(path);require(size in 0..256L*1024*1024);total=Math.addExact(total,size);require(total<=1024L*1024*1024)
                val digest=MessageDigest.getInstance("SHA-256")
                Files.newInputStream(path).use { input -> var read=0L;while(true){val n=input.read(buffer);if(n<0)break;read+=n;require(read<=size);digest.update(buffer,0,n)};require(read==size) }
                require(digest.digest().joinToString(""){"%02x".format(it)}==expected)
                progress?.verifiedFile(size)
            }
            // The same inventory must still hold after hashing.
            require(storedFiles(base)==trustedHashes.keys)
            return McpRuntime(base)
        }
        /** Relative '/' names of every regular file below [base], as the
         * directory entries store them. Attributes are the entries' own
         * (links are never followed); anything other than a plain directory
         * or plain regular file is refused, as is a directory at [MAX_DEPTH]. */
        internal fun storedFiles(base:Path):Set<String> {
            val files=HashSet<String>();var count=0
            Files.walkFileTree(base,emptySet(),MAX_DEPTH,object:SimpleFileVisitor<Path>(){
                override fun preVisitDirectory(dir:Path,attrs:BasicFileAttributes):FileVisitResult {
                    require(++count<=MAX_ENTRIES && attrs.isDirectory && !attrs.isSymbolicLink && !attrs.isOther)
                    require(dir.nameCount-base.nameCount<MAX_DEPTH)
                    return FileVisitResult.CONTINUE
                }
                override fun visitFile(file:Path,attrs:BasicFileAttributes):FileVisitResult {
                    require(++count<=MAX_ENTRIES && attrs.isRegularFile && !attrs.isSymbolicLink && !attrs.isOther)
                    require(files.add(base.relativize(file).toString().replace('\\','/')))
                    return FileVisitResult.CONTINUE
                }
                override fun visitFileFailed(file:Path,exc:IOException):FileVisitResult=throw exc
            })
            return files
        }
        private fun noRedirect(path:Path) {
            var current=path.root ?: error("Absolute runtime required")
            for(part in path){current=current.resolve(part);require(!Files.isSymbolicLink(current));require(current.toRealPath()==current.toAbsolutePath().normalize())}
        }
    }
}
