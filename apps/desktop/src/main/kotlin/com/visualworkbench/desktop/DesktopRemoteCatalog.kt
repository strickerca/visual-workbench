package com.visualworkbench.desktop

import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest

/** Package-owned catalog bytes only. Native current image/package/tool evidence
 * decides permissions. This owner is retained by the native runtime until its
 * actual remote-helper retirement fence succeeds. */
internal class DesktopRemoteCatalog private constructor(
    val path:Path, val sha256:String, private val leases:List<AutoCloseable>,
):AutoCloseable {
    private var closed=false
    @Synchronized override fun close() {
        if(closed)return
        // Keep this owner retryable if a lease cannot close.
        leases.asReversed().forEach{it.close()}
        closed=true
    }
    companion object {
        internal const val RESOURCE="vw-remote-editor-catalog.json"
        internal const val MANIFEST="vw-remote-editor-catalog.sha256"
        private const val LIMIT=32*1024
        private const val OWNER="VisualWorkbench remote editor catalog v1\n"
        internal fun prepare(base:Path,resources:NativeResources,guard:NativeFileGuard):DesktopRemoteCatalog? {
            val manifest=resources.open(MANIFEST)?.use{it.readNBytes(257)}
            val payload=resources.open(RESOURCE)?.use{it.readNBytes(LIMIT+1)}
            if(manifest==null&&payload==null)return null
            if(manifest==null||payload==null)throw NativeRuntimeFailure("The packaged editor catalog inventory is incomplete.")
            val record=record(manifest)
            if(payload.size!=record.second||sha(payload)!=record.first)
                throw NativeRuntimeFailure("The packaged editor catalog failed its exact hash check.")
            val leases=mutableListOf<AutoCloseable>()
            var created:Path?=null
            try {
                val root=base.toAbsolutePath().normalize()
                for(part in generateSequence(root){it.parent}.toList().asReversed()) {
                    if(!Files.exists(part,LinkOption.NOFOLLOW_LINKS))try{Files.createDirectory(part)}catch(_:FileAlreadyExistsException){}
                    guard.check(part)
                    if(!Files.isDirectory(part,LinkOption.NOFOLLOW_LINKS))throw NativeRuntimeFailure("An editor catalog parent is not a directory.")
                    leases+=guard.pinDirectory(part)
                    if(part.toRealPath()!=part)throw NativeRuntimeFailure("The editor catalog namespace changed.")
                }
                val key=sha(manifest);val directory=root.resolve(key)
                contained(root,directory)
                if(!Files.exists(directory,LinkOption.NOFOLLOW_LINKS)) {
                    Files.newDirectoryStream(root).use{stream->
                        var count=0;val iterator=stream.iterator();while(iterator.hasNext()){iterator.next();count++;if(count>=16)throw NativeRuntimeFailure("The editor catalog cache is full.")}
                    }
                    try{Files.createDirectory(directory);created=directory}catch(_:FileAlreadyExistsException){}
                }
                guard.check(directory);leases+=guard.pinDirectory(directory)
                if(directory.toRealPath()!=directory)throw NativeRuntimeFailure("The editor catalog directory is redirected.")
                if(created!=null) {
                    write(directory.resolve("owner"),(OWNER+key+"\n").toByteArray(Charsets.US_ASCII))
                    write(directory.resolve(RESOURCE),payload)
                    write(directory.resolve("ready"),manifest)
                }
                val expected=setOf("owner","ready",RESOURCE)
                val actual=Files.newDirectoryStream(directory).use{stream->
                    val names=mutableSetOf<String>();for(file in stream){if(names.size>=expected.size)throw NativeRuntimeFailure("Unknown editor catalog cache files.");names+=file.fileName.toString()};names
                }
                if(actual!=expected)throw NativeRuntimeFailure("The editor catalog cache is incomplete.")
                for(name in expected) {
                    val file=directory.resolve(name);contained(directory,file);guard.check(file);leases+=guard.pin(file)
                    val info=Files.readAttributes(file,BasicFileAttributes::class.java,LinkOption.NOFOLLOW_LINKS)
                    if(!info.isRegularFile||info.isSymbolicLink||info.isOther)throw NativeRuntimeFailure("The editor catalog file layout changed.")
                }
                if(!read(directory.resolve("owner"),256).contentEquals((OWNER+key+"\n").toByteArray(Charsets.US_ASCII))||
                    !read(directory.resolve("ready"),256).contentEquals(manifest))throw NativeRuntimeFailure("The editor catalog ownership marker changed.")
                val file=directory.resolve(RESOURCE);val current=read(file,LIMIT)
                if(current.size!=record.second||sha(current)!=record.first)throw NativeRuntimeFailure("The cached editor catalog changed.")
                return DesktopRemoteCatalog(file,record.first,leases.toList())
            } catch(error:Exception) {
                leases.asReversed().forEach{runCatching{it.close()}}
                // Preserve partial/unknown cache bytes. No pathname deletion is
                // attempted after releasing the strong namespace leases. This
                // failed call publishes no catalog owner or permissions.
                if(error is NativeRuntimeFailure)throw error
                throw NativeRuntimeFailure("The packaged editor catalog could not be retained safely.")
            }
        }
        private fun record(bytes:ByteArray):Pair<String,Int> {
            if(bytes.size !in 1..256||bytes.any{it.toInt() !in 10..126||it.toInt() in 11..31})throw NativeRuntimeFailure("Invalid editor catalog inventory.")
            val matched=Regex("([a-f0-9]{64}) ([1-9][0-9]{0,4}) vw-remote-editor-catalog\\.json\n").matchEntire(bytes.toString(Charsets.US_ASCII))
                ?:throw NativeRuntimeFailure("Invalid editor catalog inventory.")
            val size=matched.groupValues[2].toInt()
            if(size !in 1..LIMIT)throw NativeRuntimeFailure("The editor catalog exceeds its bound.")
            return matched.groupValues[1] to size
        }
        private fun read(path:Path,limit:Int):ByteArray=Files.newInputStream(path).use{stream->
            stream.readNBytes(limit+1).also{if(it.size>limit)throw NativeRuntimeFailure("An editor catalog file grew beyond its bound.")}
        }
        private fun write(path:Path,data:ByteArray) {
            FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{channel->
                val buffer=ByteBuffer.wrap(data);while(buffer.hasRemaining())channel.write(buffer);channel.force(true)
            }
        }
        private fun contained(parent:Path,child:Path) {
            if(child.toAbsolutePath().normalize()!=child||child.parent!=parent)throw NativeRuntimeFailure("The editor catalog path escaped its owner.")
        }
        private fun sha(bytes:ByteArray):String=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}
    }
}
