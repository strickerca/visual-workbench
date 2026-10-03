package com.visualworkbench.shared

/** JNA resolves the packaged win32-x86-64/vw_core.dll resource itself. */
internal actual fun prepareCoreLibrary() {
    check(System.getProperty("os.name").startsWith("Windows")) { "Desktop core currently targets Windows" }
    check(System.getProperty("os.arch") in setOf("amd64","x86_64")) { "Desktop core requires x86-64" }
    check(object {}.javaClass.classLoader.getResource("win32-x86-64/vw_core.dll") != null) { "Packaged core library missing; run the native packaging task" }
}
