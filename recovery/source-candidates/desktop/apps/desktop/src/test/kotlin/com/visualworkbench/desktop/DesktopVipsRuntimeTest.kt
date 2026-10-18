package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.nio.file.Files
import java.nio.file.Path

class DesktopVipsRuntimeTest {
    @get:Rule val temporary=TemporaryFolder()
    private class Guard:NativeFileGuard {
        var live=0
        override fun check(path:Path){check(Files.exists(path)&&!Files.isSymbolicLink(path))}
        override fun pin(path:Path):AutoCloseable {live++;return AutoCloseable{live--}}
        override fun pinDirectory(path:Path):AutoCloseable=pin(path)
    }
    @Test fun approvedRuntimeInventoryIsCompleteAndFlat(){
        assertEquals(38,DesktopVipsRuntime.pins.size)
        assertTrue(DesktopVipsRuntime.pins.keys.all{it.endsWith(".dll")&&!it.contains('/')&&!it.contains('\\')})
        assertTrue(DesktopVipsRuntime.pins.values.all{Regex("[0-9a-f]{64}").matches(it)})
    }
    @Test fun failedExtractionReleasesPinsButPreservesOwnedIncompleteBytes(){
        val root=temporary.newFolder().toPath().toRealPath();val guard=Guard()
        repeat(2){
            try{DesktopVipsRuntime.prepare(root,NativeResources{null},guard);fail("expected missing runtime")}
            catch(_:NativeRuntimeFailure){}
            assertEquals(0,guard.live)
        }
        val cache=Files.list(root.resolve("vips-runtime")).use{it.toList().single()}
        assertTrue(Files.isRegularFile(cache.resolve("owner")))
        assertFalse(Files.exists(cache.resolve("ready")))
        assertTrue(Files.isDirectory(cache.resolve("dll")))
    }
}
