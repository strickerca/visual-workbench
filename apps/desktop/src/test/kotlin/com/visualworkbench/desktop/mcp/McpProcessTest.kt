package com.visualworkbench.desktop.mcp

import org.junit.Assert.*
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.IOException
import java.io.OutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

class McpProcessTest {
    /** Models Process pipe synchronization deterministically: close must acquire
     * the same monitor held by a blocked write, and only child death releases it. */
    private class BlockedProcess : Process() {
        val writing=CountDownLatch(1)
        private val terminated=CountDownLatch(1)
        private val alive=AtomicBoolean(true)
        val closed=AtomicBoolean(false)
        private val pipe=object:OutputStream(){
            @Synchronized override fun write(value:Int) {
                writing.countDown();terminated.await();throw IOException("synthetic child terminated")
            }
            @Synchronized override fun close(){check(!alive.get());closed.set(true)}
        }
        override fun getOutputStream()=pipe
        override fun getInputStream()=ByteArrayInputStream(byteArrayOf())
        override fun getErrorStream()=ByteArrayInputStream(byteArrayOf())
        override fun waitFor():Int{terminated.await();return 1}
        override fun waitFor(timeout:Long,unit:TimeUnit)=terminated.await(timeout,unit)
        override fun exitValue():Int{if(alive.get())throw IllegalThreadStateException();return 1}
        override fun isAlive()=alive.get()
        override fun destroy(){alive.set(false);terminated.countDown()}
        override fun destroyForcibly():Process{destroy();return this}
    }
    @Test(timeout=10_000) fun shutdownTerminatesBeforeClosingABlockedInheritedInputPipe() {
        val process=BlockedProcess()
        val writer=Thread{runCatching{process.outputStream.write(1)}}.apply{isDaemon=true;start()}
        try {
            assertTrue(process.writing.await(5,TimeUnit.SECONDS))
            stopMcpProcess(process)
            writer.join(1000)
            assertFalse(process.isAlive);assertFalse(writer.isAlive);assertTrue(process.closed.get())
        } finally {process.destroyForcibly();writer.join(1000)}
    }
}
