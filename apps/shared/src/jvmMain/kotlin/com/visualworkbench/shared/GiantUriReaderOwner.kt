package com.visualworkbench.shared

import java.io.InputStream

/** The fixed Android bridge owns this holder before source open. A failed close
 * keeps the same stream and remains retryable; no logical seal credits native
 * retirement. Native's strong holder reference retains this entire object. */
internal class GiantUriReaderOwner(private val openSource: () -> InputStream) {
    private var attempted: Boolean = false
    private var reader: InputStream? = null

    @Synchronized fun open(): Unit {
        check(!attempted)
        attempted = true
        reader = openSource()
    }

    @Synchronized fun read(bytes: ByteArray): Int {
        require(bytes.size == 64 * 1024)
        return checkNotNull(reader).read(bytes)
    }

    @Synchronized fun close(): Unit {
        reader?.close()
        reader = null
    }
}
