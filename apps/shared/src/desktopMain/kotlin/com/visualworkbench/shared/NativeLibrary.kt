package com.visualworkbench.shared

/** JNA resolves the packaged native resource for the current test/build host.
 * Linux core validation does not enable Windows capture or input providers. */
internal actual fun prepareCoreLibrary() {
    val resource = when {
        System.getProperty("os.name").startsWith("Windows", ignoreCase = true) -> "win32-x86-64/vw_core.dll"
        System.getProperty("os.name").startsWith("Linux", ignoreCase = true) -> "linux-x86-64/libvw_core.so"
        else -> error("Desktop core validation supports Windows/Linux build hosts")
    }
    check(System.getProperty("os.arch").lowercase() in setOf("amd64","x86_64")) { "Desktop core requires x86-64" }
    check(object {}.javaClass.classLoader.getResource(resource) != null) { "Packaged core library missing; run the native packaging task" }
}
