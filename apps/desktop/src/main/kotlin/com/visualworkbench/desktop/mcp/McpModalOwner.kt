package com.visualworkbench.desktop.mcp

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.util.concurrent.atomic.AtomicBoolean

internal data class McpModalState<T>(val settings:Boolean=false,val compare:T?=null)

/** The input fence is acquired by the event handler BEFORE the panel becomes
 * observable. Compose's next frame is not a safe admission boundary. */
internal class McpModalOwner<T>(private val acquire:()->AutoCloseable):AutoCloseable {
    private val lock=Any()
    private val mutable=MutableStateFlow(McpModalState<T>())
    val state:StateFlow<McpModalState<T>> = mutable.asStateFlow()
    private var lease:AutoCloseable?=null
    private var retained=0
    private var closed=false
    fun settings()=publish(McpModalState(settings=true))
    fun compare(receipt:T)=publish(McpModalState(compare=receipt))
    fun dismiss()=publish(McpModalState())
    private fun publish(next:McpModalState<T>){synchronized(lock){
        check(!closed)
        if(next.settings||next.compare!=null){if(lease==null)lease=acquire()}
        mutable.value=next
        releaseIfIdle()
    }}
    /** A native chooser's nested event loop can dismiss its source panel. Hold
     * the same fence through that chooser and any producer borrowing its result. */
    fun retain():AutoCloseable=synchronized(lock){
        check(!closed && retained<4)
        if(lease==null)lease=acquire()
        retained++
        val released=AtomicBoolean()
        AutoCloseable{if(released.compareAndSet(false,true))synchronized(lock){retained--;releaseIfIdle()}}
    }
    private fun releaseIfIdle(){
        if(retained==0&&!mutable.value.settings&&mutable.value.compare==null){val old=lease;lease=null;old?.close()}
    }
    override fun close(){synchronized(lock){if(closed)return;closed=true;mutable.value=McpModalState();releaseIfIdle()}}
}
