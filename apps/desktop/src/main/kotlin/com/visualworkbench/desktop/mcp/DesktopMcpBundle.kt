package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.*
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

/** Server-only resources avoid an app-JAR/self-hash cycle. The final installed
 * bridge independently pins the completed app image before minimized launch. */
internal class DesktopMcpBundle private constructor(
    val runtime: McpRuntime, private val pins: List<AutoCloseable>,
) : AutoCloseable {
    private val closed=AtomicBoolean()
    override fun close(){if(closed.compareAndSet(false,true))pins.asReversed().forEach{runCatching{it.close()}}}
    companion object {
        internal const val MANIFEST="vw-mcp-server.sha256"
        internal const val MAX_MANIFEST=4*1024*1024
        internal data class Entry(val hash:String,val bytes:Long,val name:String)
        internal fun parse(bytes:ByteArray):List<Entry>{
            require(bytes.size in 1..MAX_MANIFEST && bytes.all{it==10.toByte() || it.toInt() in 32..126})
            val seen=HashSet<String>();var total=0L
            val entries=bytes.toString(Charsets.US_ASCII).lineSequence().filter(String::isNotEmpty).map{line->
                require(seen.size<20_000)
                val part=line.split(' ',limit=3);require(part.size==3 && part[0].matches(Regex("[0-9a-f]{64}")))
                val size=part[1].toLongOrNull()?:throw McpRefused();require(size in 0..256L*1024*1024)
                val name=part[2];require(name.length in 1..256 && name.split('/').size<=32 && name.split('/').all{segment->
                    segment.isNotEmpty() && segment !in listOf(".","..") && segment.all{it.isLetterOrDigit() || it in "._-@+"} &&
                        !segment.endsWith('.') && segment.substringBefore('.').uppercase() !in setOf("CON","PRN","AUX","NUL","COM1","COM2","COM3","COM4","COM5","COM6","COM7","COM8","COM9","LPT1","LPT2","LPT3","LPT4","LPT5","LPT6","LPT7","LPT8","LPT9")
                })
                require(seen.add(name.lowercase()))
                total=Math.addExact(total,size);require(total<=1024L*1024*1024)
                Entry(part[0],size,name)
            }.toList()
            val names=entries.map{it.name}.toSet()
            require(names.containsAll(listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs","vw-codex-host.exe", "mcp/codex/main.mjs", "mcp/codex/client.mjs", "mcp/codex/package.mjs", "mcp/codex/process.mjs", "mcp/codex/schema.mjs", "mcp/codex/schema-worker.mjs", "mcp/codex/image-profiles.json")))
            require(names.none{it=="vw-mcp.exe" || it.endsWith(".jar") || it=="VisualWorkbench.exe" || it=="VisualWorkbenchDev.exe"})
            return entries
        }
        fun prepare(privateParent:Path):DesktopMcpBundle=prepare(privateParent,
            NativeResources{DesktopMcpBundle::class.java.classLoader.getResourceAsStream(it)},WindowsNativeFileGuard())
        internal fun prepare(parent:Path,resources:NativeResources,guard:NativeFileGuard):DesktopMcpBundle{
            val pins=mutableListOf<AutoCloseable>()
            var stage=McpFailureStage.bundle_parents
            try{
                require(parent.isAbsolute && parent==parent.normalize())
                for(ancestor in generateSequence(parent){it.parent}.toList().asReversed()){
                    guard.check(ancestor);require(Files.isDirectory(ancestor,LinkOption.NOFOLLOW_LINKS) && ancestor.toRealPath()==ancestor)
                    pins+=guard.pinDirectory(ancestor)
                }
                stage=McpFailureStage.bundle_manifest
                val manifest=resources.open(MANIFEST)?.use{it.readNBytes(MAX_MANIFEST+1)}?:throw McpRefused()
                val entries=parse(manifest);val key=hex(MessageDigest.getInstance("SHA-256").digest(manifest))
                stage=McpFailureStage.bundle_extract
                val base=parent.resolve("mcp-server-bundles-v1")
                directory(base,guard,pins)
                val folder=base.resolve(key)
                if(!Files.exists(folder,LinkOption.NOFOLLOW_LINKS)){
                    Files.newDirectoryStream(base).use{stream->require(stream.take(9).count()<8)}
                    Files.createDirectory(folder);guard.check(folder);pins+=guard.pinDirectory(folder)
                    write(folder.resolve("owner"),("VisualWorkbench MCP server v1\n"+key+"\n").toByteArray())
                    val runtime=folder.resolve("runtime");directory(runtime,guard,pins)
                    val made=HashSet<Path>();made.add(runtime)
                    for(entry in entries){
                        val target=runtime.resolve(entry.name).normalize();require(target.startsWith(runtime))
                        for(dir in generateSequence(target.parent){if(it==runtime)null else it.parent}.toList().asReversed())if(made.add(dir))directory(dir,guard,pins)
                        resources.open("mcp-server/${entry.name}")?.use{input->
                            FileChannel.open(target,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{output->
                                val hash=MessageDigest.getInstance("SHA-256");val bytes=ByteArray(65536);var total=0L
                                while(true){val n=input.read(bytes);if(n<0)break;require(n>0);total=Math.addExact(total,n.toLong());require(total<=entry.bytes);hash.update(bytes,0,n)
                                    val buffer=ByteBuffer.wrap(bytes,0,n);while(buffer.hasRemaining())output.write(buffer)}
                                require(total==entry.bytes && hex(hash.digest())==entry.hash);output.force(true)
                            }
                        }?:throw McpRefused()
                    }
                    write(folder.resolve("ready"),manifest)
                }else{guard.check(folder);pins+=guard.pinDirectory(folder)}
                stage=McpFailureStage.bundle_markers
                require(Files.newDirectoryStream(folder).use{it.take(4).map{p->p.fileName.toString()}.toSet()}==setOf("owner","ready","runtime"))
                for(marker in listOf("owner","ready")){val path=folder.resolve(marker);guard.check(path);pins+=guard.pin(path)}
                require(read(folder.resolve("owner"),256).contentEquals(("VisualWorkbench MCP server v1\n"+key+"\n").toByteArray()))
                require(read(folder.resolve("ready"),MAX_MANIFEST).contentEquals(manifest))
                stage=McpFailureStage.bundle_inventory
                val runtime=folder.resolve("runtime");val expected=entries.associateBy{it.name};val actual=HashSet<String>();var visited=0
                Files.walk(runtime,33).use{walk->walk.forEach{path->
                    require(++visited<=30_000);guard.check(path);require(path.toRealPath()==path)
                    if(Files.isDirectory(path,LinkOption.NOFOLLOW_LINKS)){pins+=guard.pinDirectory(path);require(path.nameCount-runtime.nameCount<=32)}else{
                        val name=runtime.relativize(path).toString().replace('\\','/');val entry=expected[name]?:throw McpRefused()
                        require(actual.add(name));pins+=guard.pin(path);require(Files.isRegularFile(path,LinkOption.NOFOLLOW_LINKS) && Files.size(path)==entry.bytes)
                    }
                }}
                require(actual==expected.keys)
                stage=McpFailureStage.bundle_verify
                val verified=McpRuntime.verify(runtime,entries.associate{it.name to it.hash},serverOnly=true)
                return DesktopMcpBundle(verified,pins.toList())
            }catch(error:Exception){pins.asReversed().forEach{runCatching{it.close()}};throw McpRefused(stage)}
            // Partial or preexisting bundles are intentionally retained; no path-based cleanup.
        }
        private fun directory(path:Path,guard:NativeFileGuard,pins:MutableList<AutoCloseable>){
            if(!Files.exists(path,LinkOption.NOFOLLOW_LINKS))Files.createDirectory(path)
            guard.check(path);require(Files.isDirectory(path,LinkOption.NOFOLLOW_LINKS) && path.toRealPath()==path);pins+=guard.pinDirectory(path)
        }
        private fun read(path:Path,limit:Int)=Files.newInputStream(path).use{it.readNBytes(limit+1).also{bytes->require(bytes.size<=limit)}}
        private fun write(path:Path,bytes:ByteArray){FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{file->val b=ByteBuffer.wrap(bytes);while(b.hasRemaining())file.write(b);file.force(true)}}
        private fun hex(bytes:ByteArray)=bytes.joinToString(""){"%02x".format(it.toInt() and 255)}
    }
}
