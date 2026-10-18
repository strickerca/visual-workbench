package com.visualworkbench.desktop.remote

import java.nio.file.Files

internal object RemoteCompatibilityFileCases {
    fun run():Int {
        val directory=Files.createTempDirectory("vw-compatibility-file-")
        try {
            val exact=directory.resolve("exact.json")
            val bytes=ByteArray(32768){32}
            Files.write(exact,bytes)
            check(readCompatibilityReceipt(exact).contentEquals(bytes))
            val large=directory.resolve("large.json");Files.write(large,ByteArray(32769))
            check(runCatching{readCompatibilityReceipt(large)}.isFailure)
            val empty=directory.resolve("empty.json");Files.write(empty,byteArrayOf())
            check(runCatching{readCompatibilityReceipt(empty)}.isFailure)
            check(runCatching{readCompatibilityReceipt(directory)}.isFailure)
            val symlink=directory.resolve("link.json");Files.createSymbolicLink(symlink,exact)
            check(runCatching{readCompatibilityReceipt(symlink)}.isFailure)
            return 5
        }finally{Files.newDirectoryStream(directory).use{it.forEach(Files::delete)};Files.delete(directory)}
    }
}
public fun main():Unit {println("Compatibility import file cases passed: ${RemoteCompatibilityFileCases.run()}")}
