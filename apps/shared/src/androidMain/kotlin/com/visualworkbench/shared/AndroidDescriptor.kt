package com.visualworkbench.shared

import android.os.ParcelFileDescriptor
import android.system.OsConstants
import com.sun.jna.Library
import com.sun.jna.Native
import java.io.IOException

/** Caller retains its open descriptor for this synchronous platform operation.
 * Android's public Os.fcntlInt starts at API 30; libc fcntl is available at the
 * app's API 29 minimum. No hidden Java API, fd reflection or ownership transfer. */
public fun setAndroidDescriptorNonBlocking(descriptor: ParcelFileDescriptor) {
    val fd = descriptor.fd
    require(fd >= 0)
    val flags = descriptorLibC.fcntl(fd, OsConstants.F_GETFL, 0)
    if (flags < 0 || descriptorLibC.fcntl(fd, OsConstants.F_SETFL, flags or OsConstants.O_NONBLOCK) < 0) {
        throw IOException("Could not configure the image provider descriptor")
    }
}

private interface DescriptorLibC : Library {
    // The true variadic declaration lets JNA use the Android ABI's vararg rules.
    fun fcntl(fd: Int, command: Int, vararg arguments: Any): Int
}

private val descriptorLibC: DescriptorLibC by lazy {
    Native.load("c", DescriptorLibC::class.java)
}
