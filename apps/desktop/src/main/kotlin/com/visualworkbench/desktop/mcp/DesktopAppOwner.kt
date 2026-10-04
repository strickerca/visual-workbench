package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.NativeFileGuard
import com.visualworkbench.desktop.WindowsNativeFileGuard
import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.sun.jna.WString
import com.sun.jna.win32.StdCallLibrary
import com.sun.nio.file.ExtendedOpenOption
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.nio.channels.FileLock
import java.nio.channels.OverlappingFileLockException
import java.nio.file.*
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean

internal enum class AppStartRequest { Show, Mcp }
internal data class AppRequest(val kind:AppStartRequest,val nonce:String,val createdMs:Long)
/** Only nonsecret commands are stored here. No project path, token, grant or
 * Send data is accepted. App ownership precedes project/session/service owners;
 * verified native loading and process DPI setup run first. */
internal class DesktopAppOwner private constructor(
    private val root:Path,private val channel:FileChannel,private val lock:FileLock,
    private val pins:List<AutoCloseable>,private val markers:AppMarkerIo,
) : AutoCloseable {
    private val closed=AtomicBoolean()
    fun poll(nowMs:Long=System.currentTimeMillis()):AppStartRequest? {
        check(!closed.get())
        return markers.consume(root.resolve("request")){bytes->
            val request=decode(bytes)
            // Structurally valid expired/future markers are retired without action.
            request.kind.takeIf{request.createdMs<=nowMs && nowMs-request.createdMs<=TTL}
        }
    }
    override fun close(){if(closed.compareAndSet(false,true)){try{lock.release()}finally{try{channel.close()}finally{pins.asReversed().forEach{runCatching{it.close()}}}}}}
    companion object {
        internal const val MAX=160
        internal const val TTL=30_000L
        internal fun encode(kind:AppStartRequest,nonce:String,nowMs:Long):ByteArray{
            require(nonce.matches(Regex("[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")) && nowMs>0)
            return "VWAPP1\n${kind.name}\n$nonce\n$nowMs\n".toByteArray(Charsets.US_ASCII)
        }
        internal fun decode(bytes:ByteArray):AppRequest{
            require(bytes.size in 1..MAX && bytes.all{it==10.toByte() || it.toInt() in 32..126})
            val parts=bytes.toString(Charsets.US_ASCII).split('\n');require(parts.size==5 && parts[0]=="VWAPP1" && parts[4].isEmpty())
            val kind=AppStartRequest.entries.singleOrNull{it.name==parts[1]}?:throw McpRefused()
            val time=parts[3].toLongOrNull()?:throw McpRefused();require(time>0)
            require(encode(kind,parts[2],time).contentEquals(bytes))
            return AppRequest(kind,parts[2],time)
        }
        fun claim(parent:Path):DesktopAppOwner?=claim(parent,WindowsNativeFileGuard(),WindowsAppMarkerIo())
        internal fun claim(parent:Path,guard:NativeFileGuard,markers:AppMarkerIo):DesktopAppOwner?{
            val pins=mutableListOf<AutoCloseable>();var file:FileChannel?=null;var lease:FileLock?=null
            try{
                val root=prepare(parent,guard,pins)
                val path=root.resolve("owner.lock")
                if(Files.exists(path,LinkOption.NOFOLLOW_LINKS))guard.check(path)
                file=FileChannel.open(path,StandardOpenOption.CREATE,StandardOpenOption.READ,StandardOpenOption.WRITE,ExtendedOpenOption.NOSHARE_DELETE,LinkOption.NOFOLLOW_LINKS)
                guard.check(path);require(file.size()==0L)
                lease=try{file.tryLock()}catch(_:OverlappingFileLockException){null}
                if(lease==null){file.close();pins.asReversed().forEach{it.close()};return null}
                return DesktopAppOwner(root,file,lease,pins.toList(),markers)
            }catch(error:Throwable){runCatching{lease?.release()};runCatching{file?.close()};pins.asReversed().forEach{runCatching{it.close()}};throw error}
        }
        fun request(parent:Path,kind:AppStartRequest):Unit=request(parent,kind,WindowsNativeFileGuard(),WindowsAppMarkerIo())
        internal fun request(parent:Path,kind:AppStartRequest,guard:NativeFileGuard,markers:AppMarkerIo){
            val pins=mutableListOf<AutoCloseable>();var stage:Path?=null;var expected:ByteArray?=null
            try{
                val root=prepare(parent,guard,pins)
                Files.newDirectoryStream(root).use{stream->require(stream.take(19).count()<18)}
                val bytes=encode(kind,UUID.randomUUID().toString(),System.currentTimeMillis());expected=bytes
                val path=root.resolve("request-write-${UUID.randomUUID()}");stage=path
                FileChannel.open(path,StandardOpenOption.CREATE_NEW,StandardOpenOption.WRITE,ExtendedOpenOption.NOSHARE_DELETE).use{out->val b=ByteBuffer.wrap(bytes);while(b.hasRemaining())out.write(b);out.force(true)}
                try{Files.createLink(root.resolve("request"),path)}catch(_:FileAlreadyExistsException){
                    // One existing bounded command is pending. It is never overwritten.
                    val pending=markers.peek(root.resolve("request"));decode(pending)
                    val existing=decode(pending);val now=System.currentTimeMillis()
                    if(existing.kind!=kind || existing.createdMs>now || now-existing.createdMs>TTL)throw McpRefused()
                }
            }finally{
                val owned=stage;val bytes=expected
                try{if(owned!=null && bytes!=null)markers.consume(owned){actual->require(actual.contentEquals(bytes));Unit}}
                finally{pins.asReversed().forEach{runCatching{it.close()}}}
            }
        }
        private fun prepare(parent:Path,guard:NativeFileGuard,pins:MutableList<AutoCloseable>):Path{
            require(parent.isAbsolute && parent==parent.normalize())
            for(part in generateSequence(parent){it.parent}.toList().asReversed()){
                guard.check(part);require(Files.isDirectory(part,LinkOption.NOFOLLOW_LINKS) && part.toRealPath()==part);pins+=guard.pinDirectory(part)
            }
            val root=parent.resolve("app-owner-v1")
            if(!Files.exists(root,LinkOption.NOFOLLOW_LINKS))try{Files.createDirectory(root)}catch(_:FileAlreadyExistsException){}
            guard.check(root);require(Files.isDirectory(root,LinkOption.NOFOLLOW_LINKS)&&root.toRealPath()==root);pins+=guard.pinDirectory(root)
            return root
        }
    }
}
internal interface AppMarkerIo {
    fun peek(path:Path):ByteArray
    /** Validate the bounded bytes while a deny-write/delete handle is retained;
     * retire only that same opened file/link after validation returns normally. */
    fun <T> consume(path:Path,validate:(ByteArray)->T):T?
}
internal class WindowsAppMarkerIo:AppMarkerIo {
    private interface Kernel:StdCallLibrary {
        fun CreateFileW(path:WString,access:Int,share:Int,security:Pointer?,disposition:Int,flags:Int,template:Pointer?):Pointer?
        fun ReadFile(handle:Pointer,bytes:ByteArray,count:Int,read:IntArray,overlapped:Pointer?):Boolean
        fun GetFileInformationByHandleEx(handle:Pointer,kind:Int,buffer:Pointer,bytes:Int):Boolean
        fun SetFileInformationByHandle(handle:Pointer,kind:Int,buffer:Pointer,bytes:Int):Boolean
        fun CloseHandle(handle:Pointer):Boolean
    }
    private val api:Kernel=Native.load("kernel32",Kernel::class.java)
    override fun peek(path:Path):ByteArray=opened(path,false){bytes,_->bytes}?:throw McpRefused()
    override fun <T> consume(path:Path,validate:(ByteArray)->T):T?=opened(path,true){bytes,handle->
        val value=validate(bytes)
        Memory(1).use{flag->flag.setByte(0,1);check(api.SetFileInformationByHandle(handle,4,flag,1))}
        value
    }
    private fun <T> opened(path:Path,delete:Boolean,body:(ByteArray,Pointer)->T):T?{
        val raw=api.CreateFileW(WString(path.toString()),0x80000000.toInt() or if(delete)0x10000 else 0,1,null,3,0x00200000,null)
        if(raw==null || Pointer.nativeValue(raw)==-1L){val error=Native.getLastError();if(error==2)return null;throw McpRefused()}
        try{
            Memory(8).use{tag->require(api.GetFileInformationByHandleEx(raw,9,tag,8));require((tag.getInt(0) and (0x400 or 0x10))==0)}
            val bytes=ByteArray(DesktopAppOwner.MAX+1);val count=intArrayOf(0)
            require(api.ReadFile(raw,bytes,bytes.size,count,null));require(count[0] in 1..DesktopAppOwner.MAX)
            // ReadFile can return a short read; local ordinary files are read
            // again to prove EOF rather than accepting a valid prefix.
            val extra=ByteArray(1);val tail=intArrayOf(0);require(api.ReadFile(raw,extra,1,tail,null)&&tail[0]==0)
            return body(bytes.copyOf(count[0]),raw)
        }finally{api.CloseHandle(raw)}
    }
}
