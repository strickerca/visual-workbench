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
        private val RESERVED=setOf("CON","PRN","AUX","NUL","COM1","COM2","COM3","COM4","COM5","COM6","COM7","COM8","COM9","LPT1","LPT2","LPT3","LPT4","LPT5","LPT6","LPT7","LPT8","LPT9")
        /** The bundle path grammar, shared with [DesktopMcpBundle.parse]:
         * 1..256 chars, at most [MAX_DEPTH] '/'-separated segments, each made
         * only of ASCII letters/digits and `._-@+`, never `.`/`..`, never ending
         * in a dot (spaces are outside the charset) and never a reserved DOS
         * device base name. */
        internal fun admissibleName(name:String):Boolean = name.length in 1..256 && name.split('/').let{parts->
            parts.size<=MAX_DEPTH && parts.all{segment->
                segment.isNotEmpty() && segment !in listOf(".","..") &&
                    segment.all{it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it in "._-@+"} &&
                    !segment.endsWith('.') && segment.substringBefore('.').uppercase() !in RESERVED
            }
        }
        /** Every call re-hashes every admitted byte; nothing is reused from an
         * earlier call. Redirection checks: one canonical toRealPath check of
         * the root, then a no-follow tree walk that reads each entry's own
         * attributes and takes its name as stored. Any reparse point, symlink,
         * special file, unlisted file or directory at the depth limit is
         * refused, as is a manifest whose names are not exactly the stored
         * names (for example two manifest names that alias one stored file).
         * The walk runs before and again after hashing.
         *
         * Files are then opened by path. Between the walk and each open, this
         * function relies on the caller's inventory pins: [DesktopMcpBundle]
         * holds no-write/no-delete handles on every runtime file and
         * directory, checked against their final paths, from before this call
         * until the bundle closes. Without such pins the walk is only a
         * point-in-time check, as the previous per-ancestor checks were.
         *
         * The previous version ran toRealPath on every ancestor of every file
         * (O(files x depth^2) directory lookups, about 60 s for the packaged
         * 1,514-file bundle). This walk is O(entries). */
        fun verify(root: Path, trustedHashes: Map<String,String>,serverOnly:Boolean=false,progress:McpStartupProgress?=null): McpRuntime {
            require(trustedHashes.size in 4..20_000)
            val base=root.toAbsolutePath().normalize()
            require(base==root && Files.isDirectory(base,LinkOption.NOFOLLOW_LINKS))
            noRedirect(base)
            val folded=HashSet<String>()
            trustedHashes.forEach { (name, expected) ->
                require(admissibleName(name) && folded.add(name.lowercase()))
                require(expected.matches(Regex("[0-9a-f]{64}")))
                require(base.resolve(name).normalize().startsWith(base))
            }
            val required=if(serverOnly)listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs") else listOf("VisualWorkbench.exe","vw-mcp-host.exe","vw-mcp.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs")
            for(name in required+listOf("vw-codex-host.exe", "mcp/codex/main.mjs", "mcp/codex/client.mjs", "mcp/codex/package.mjs", "mcp/codex/process.mjs", "mcp/codex/schema.mjs", "mcp/codex/schema-worker.mjs", "mcp/codex/image-profiles.json")) require(trustedHashes.containsKey(name))
            // Node searches dependencies on disk: an unlisted injection cannot be
            // admitted alongside valid pins. This directory is a dedicated bundle.
            // Exact (case-sensitive) set equality with the stored names means
            // each manifest name is the stored name, not a second alias of it.
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
         * directory entries store them. Each entry's attributes are read again
         * from the entry itself (links never followed) rather than taken from
         * the directory listing: anything other than a plain directory or a
         * plain regular file is refused. With maxDepth = [MAX_DEPTH], a
         * directory at that depth is delivered to visitFile and refused there
         * as not a regular file; deeper entries are never reached. */
        internal fun storedFiles(base:Path):Set<String> {
            val files=HashSet<String>();var count=0
            fun own(path:Path)=Files.readAttributes(path,BasicFileAttributes::class.java,LinkOption.NOFOLLOW_LINKS)
            Files.walkFileTree(base,emptySet(),MAX_DEPTH,object:SimpleFileVisitor<Path>(){
                override fun preVisitDirectory(dir:Path,attrs:BasicFileAttributes):FileVisitResult {
                    val actual=own(dir)
                    require(++count<=MAX_ENTRIES && actual.isDirectory && !actual.isSymbolicLink && !actual.isOther)
                    return FileVisitResult.CONTINUE
                }
                override fun visitFile(file:Path,attrs:BasicFileAttributes):FileVisitResult {
                    val actual=own(file)
                    require(++count<=MAX_ENTRIES && actual.isRegularFile && !actual.isSymbolicLink && !actual.isOther)
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
