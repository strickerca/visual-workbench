package com.visualworkbench.desktop.documents

import com.visualworkbench.desktop.*
import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.file.*
import java.security.MessageDigest

/** Only hashes compiled into this application's resource set are trusted.
 * PinnedRuntime independently repeats the exact byte/name/ancestor audit. */
internal object DesktopDocumentRuntimeCache {
    suspend fun prepare(local: Path): DocumentJobs {
        val loader=DesktopDocumentRuntimeCache::class.java.classLoader
        val bytes=loader.getResourceAsStream("vw-document-runtime.sha256")?.use{it.readNBytes(8193)} ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
        if(bytes.size !in 1..8192 || bytes.any{it.toInt() !in 10..126 || it.toInt() in 11..31}) throw DocumentFailure(DocumentFailureKind.Verification)
        val pattern=Regex("([0-9a-f]{64}) ([1-9][0-9]{0,8}) (pdfium\\.dll|qpdf30\\.dll|vw-document-worker\\.exe)")
        val entries=bytes.toString(Charsets.US_ASCII).lineSequence().filter(String::isNotEmpty).map{line ->
            val match=pattern.matchEntire(line) ?: throw DocumentFailure(DocumentFailureKind.Verification)
            val size=match.groupValues[2].toLong();if(size>256L*1024*1024) throw DocumentFailure(DocumentFailureKind.Limit)
            Triple(match.groupValues[3],size,match.groupValues[1])
        }.toList()
        if(entries.size !in 2..3 || entries.map{it.first}.toSet().size!=entries.size || !entries.any{it.first=="pdfium.dll"} || !entries.any{it.first=="vw-document-worker.exe"}) throw DocumentFailure(DocumentFailureKind.Verification)
        val key=hex(MessageDigest.getInstance("SHA-256").digest(bytes))
        val guard=WindowsNativeFileGuard();val leases=mutableListOf<AutoCloseable>();var opened:DesktopDocumentRuntime?=null
        try {
            val anchor=NativeCacheAnchor.prepare(local,guard);leases+=anchor
            val base=anchor.directory.resolve("document-runtime");if(!Files.exists(base,LinkOption.NOFOLLOW_LINKS)) Files.createDirectory(base)
            guard.check(base);leases+=guard.pinDirectory(base)
            val scratch=base.resolve("document-worker-private");if(!Files.exists(scratch,LinkOption.NOFOLLOW_LINKS)) Files.createDirectory(scratch)
            guard.check(scratch);leases+=guard.pinDirectory(scratch)
            val directory=base.resolve(key)
            var created=false
            try { Files.createDirectory(directory);created=true } catch(_:FileAlreadyExistsException) {}
            guard.check(directory);leases+=guard.pinDirectory(directory)
            if(created) for((name,size,digest) in entries) {
                val path=directory.resolve(name)
                loader.getResourceAsStream("document-win32-x86-64/$name")?.use{input ->
                    FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE).use{output ->
                        val hash=MessageDigest.getInstance("SHA-256");val buffer=ByteArray(65536);var total=0L
                        while(true) { val count=input.read(buffer);if(count<0) break;if(count==0) throw DocumentFailure(DocumentFailureKind.Verification);total+=count;if(total>size) throw DocumentFailure(DocumentFailureKind.Verification);hash.update(buffer,0,count);val chunk=ByteBuffer.wrap(buffer,0,count);while(chunk.hasRemaining()) output.write(chunk) }
                        if(total!=size || hex(hash.digest())!=digest) throw DocumentFailure(DocumentFailureKind.Verification);output.force(true)
                    }
                } ?: throw DocumentFailure(DocumentFailureKind.Unavailable)
            }
            // Partial extraction and existing unknown entries are preserved and
            // refused; never recursively delete/adopt an incomplete cache.
            val manifest=DocumentRuntimeOptions(directory.toString(),entries.map{DocumentRuntimeArtifact(it.first,it.second.toULong(),unhex(it.third))})
            val runtime=openDesktopDocumentRuntime(manifest);opened=runtime
            anchor.verify()
            return DesktopDocumentJobs(runtime,entries.any{it.first=="qpdf30.dll"},AutoCloseable{leases.asReversed().forEach{it.close()}})
        } catch(error: Exception) {opened?.let{runCatching{it.destroy()}};leases.asReversed().forEach{runCatching{it.close()}};if(error is DocumentJobException) throw nativeDocumentFailure(error);throw error }
    }
    private fun hex(bytes: ByteArray): String = bytes.joinToString(""){"%02x".format(it.toInt() and 255)}
    private fun unhex(text: String): ByteArray = text.chunked(2).map{it.toInt(16).toByte()}.toByteArray()
}
