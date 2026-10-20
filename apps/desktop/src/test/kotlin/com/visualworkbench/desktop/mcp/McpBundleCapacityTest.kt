package com.visualworkbench.desktop.mcp

import com.visualworkbench.desktop.NativeFileGuard
import com.visualworkbench.desktop.NativeResources
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.Path
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.*
import org.junit.Test

/** Synthetic resources/leases only; never touches a user's cache or starts a process. */
class McpBundleCapacityTest {
    @Test fun eightEntriesRefuseAnAbsentKeyWithoutChangingThemOrOpeningPayloads() {
        McpCapacityFixture().use { fixture ->
            fixture.fill(8)
            val before=fixture.inventory()
            val progress=McpStartupProgress{0L};progress.begin()
            val refusal=mcpCapacityRefusal{fixture.prepare(progress).close()}
            assertEquals(McpFailureStage.bundle_extract,refusal.stage)
            assertEquals(McpRefusalDetail.cache_capacity,refusal.detail)
            assertEquals(before,fixture.inventory())
            assertFalse(Files.exists(fixture.bundle,LinkOption.NOFOLLOW_LINKS))
            assertEquals(0,fixture.payloadOpens)
            assertEquals(0,fixture.guard.held)
            assertEquals(McpStartupPhase.bundle_extract,progress.snapshot().phase)
            assertEquals(McpCapacityFixture.names.size,progress.snapshot().files)
            assertEquals(0,progress.snapshot().verifiedFiles)
        }
    }

    @Test fun sevenEntriesAllowExactlyTheEighthBundleWithoutEvictingAnything() {
        McpCapacityFixture().use { fixture ->
            fixture.fill(7)
            val before=fixture.inventory()
            fixture.prepare().use { bundle ->
                assertEquals(fixture.bundle.resolve("runtime"),bundle.runtime.root)
                assertEquals(8,Files.newDirectoryStream(fixture.base).use{it.count()})
                assertTrue(Files.readAllBytes(fixture.bundle.resolve("ready")).contentEquals(fixture.manifest))
                assertEquals(McpCapacityFixture.names.size,fixture.payloadOpens)
                val after=fixture.inventory()
                for((name,value) in before)assertEquals(value,after[name])
                assertTrue(fixture.guard.held>0)
            }
            assertEquals(0,fixture.guard.held)
        }
    }

    @Test fun anExistingVerifiedKeyAtCapacityReusesItsBytesAndRetainsBothOwnersLeases() {
        McpCapacityFixture().use { fixture ->
            fixture.prepare().use { first ->
                fixture.fill(7)
                val before=fixture.inventory()
                val firstHeld=fixture.guard.held
                val firstOpens=fixture.payloadOpens
                fixture.prepare().use { second ->
                    assertEquals(first.runtime.root,second.runtime.root)
                    assertEquals(before,fixture.inventory())
                    assertEquals(firstOpens,fixture.payloadOpens)
                    assertTrue(fixture.guard.held>firstHeld)
                }
                assertEquals(firstHeld,fixture.guard.held)
                assertEquals(before,fixture.inventory())
            }
            assertEquals(0,fixture.guard.held)
        }
    }

    @Test fun capacityRefusalForANewBuildDoesNotReleaseAnActiveBuildsLeases() {
        McpCapacityFixture().use { fixture ->
            fixture.prepare().use {
                fixture.fill(7)
                val before=fixture.inventory()
                val held=fixture.guard.held
                val opens=fixture.payloadOpens
                val otherManifest=fixture.manifest.toString(Charsets.US_ASCII).lineSequence()
                    .filter{it.isNotEmpty()}.toList().reversed().joinToString("\n",postfix="\n").toByteArray(Charsets.US_ASCII)
                val otherResources=NativeResources { name ->
                    if(name==DesktopMcpBundle.MANIFEST)otherManifest.inputStream()
                    else throw AssertionError("Capacity must refuse before opening a payload")
                }
                val refusal=mcpCapacityRefusal{DesktopMcpBundle.prepare(fixture.parent,otherResources,fixture.guard).close()}
                assertEquals(McpRefusalDetail.cache_capacity,refusal.detail)
                assertEquals(held,fixture.guard.held)
                assertTrue(held>0)
                assertEquals(opens,fixture.payloadOpens)
                assertEquals(before,fixture.inventory())
            }
            assertEquals(0,fixture.guard.held)
        }
    }

    @Test fun aPreexistingIncompleteKeyAtCapacityStillReportsItsRealMarkerFailure() {
        McpCapacityFixture().use { fixture ->
            fixture.fill(7)
            Files.createDirectory(fixture.bundle)
            Files.writeString(fixture.bundle.resolve("preserve.txt"),"synthetic incomplete key")
            val before=fixture.inventory()
            val refusal=mcpCapacityRefusal{fixture.prepare().close()}
            assertEquals(McpFailureStage.bundle_markers,refusal.stage)
            assertNull(refusal.detail)
            assertEquals(before,fixture.inventory())
            assertEquals(0,fixture.payloadOpens)
            assertEquals(0,fixture.guard.held)
        }
    }

    @Test fun aResourceRefusalCannotImpersonateTheOwnedCapacityCheck() {
        for(stage in listOf(McpFailureStage.bundle_extract,McpFailureStage.catalog)) {
            McpCapacityFixture().use { fixture ->
                val resources=NativeResources { name ->
                    if(name==DesktopMcpBundle.MANIFEST)fixture.manifest.inputStream()
                    else throw McpRefused(stage,McpRefusalDetail.cache_capacity)
                }
                val refusal=mcpCapacityRefusal{DesktopMcpBundle.prepare(fixture.parent,resources,fixture.guard).close()}
                assertEquals(McpFailureStage.bundle_extract,refusal.stage)
                assertNull(refusal.detail)
                assertFalse(Files.exists(fixture.bundle.resolve("ready"),LinkOption.NOFOLLOW_LINKS))
                assertEquals(0,fixture.guard.held)
            }
        }
    }

    @Test fun capacityDetailReachesTheExistingAppIssueWithoutChangingSmokeGrammar()=runBlocking {
        withTimeout(3000) {
            McpCapacityFixture().use { fixture ->
                fixture.fill(8)
                val before=fixture.inventory()
                val lifetime=DesktopMcpLifetime {
                    fixture.prepare().close()
                    throw AssertionError("An absent key must not start at capacity")
                }
                try {
                    lifetime.start()
                    val state=lifetime.state.value
                    assertFalse(state.starting);assertFalse(state.closed);assertNull(state.owner)
                    assertEquals(McpFailureStage.bundle_extract,state.failureStage)
                    assertEquals(McpRefusalDetail.cache_capacity,state.failureDetail)
                    assertEquals(
                        "The local MCP runtime cache has reached its limit of 8 entries. Close every Workbench instance and agent handoff, then ask the build owner to review cache ownership and use before removing anything. Retry after a reviewed slot is available. Rebuilding alone does not free a cache slot.",
                        state.issue,
                    )
                    val smoke=McpSmokeProgress(diagnostic={})
                    smoke.at(McpSmokeStep.first_start){}
                    assertEquals("VW_DESKTOP_MCP failed=true stage=bundle_extract step=first_start reason=refused",
                        smoke.failureMarker(McpRefused(checkNotNull(state.failureStage),state.failureDetail)))
                    assertEquals(before,fixture.inventory());assertEquals(0,fixture.payloadOpens)
                    assertEquals(0,fixture.guard.held)
                } finally {lifetime.close()}
            }
        }
    }

    @Test fun genericOrMismatchedFailuresKeepGenericGuidanceAndNoCapacityDetail()=runBlocking {
        withTimeout(3000) {
            for(failure in listOf(McpRefused(McpFailureStage.bundle_extract),
                McpRefused(McpFailureStage.catalog,McpRefusalDetail.cache_capacity))) {
                val lifetime=DesktopMcpLifetime{throw failure}
                try {
                    lifetime.start()
                    val state=lifetime.state.value
                    assertEquals(failure.stage,state.failureStage)
                    assertNull(state.failureDetail)
                    assertEquals("The packaged MCP service is unavailable or its runtime inventory failed verification. Rebuild the complete MCP package; no alternate listener was started.",state.issue)
                    assertFalse(state.starting);assertNull(state.owner)
                } finally {lifetime.close()}
            }
        }
    }
}

private fun mcpCapacityRefusal(body:()->Unit):McpRefused {
    try{body()}catch(error:McpRefused){return error}
    throw AssertionError("Expected a bounded MCP refusal")
}

private class McpCapacityFixture:AutoCloseable {
    val parent:Path=Files.createTempDirectory("vw-mcp-capacity-").toRealPath()
    val base:Path=parent.resolve("mcp-server-bundles-v1")
    val guard=McpCapacityGuard()
    val manifest:ByteArray=names.joinToString(""){"${hash(byteArrayOf(1))} 1 $it\n"}.toByteArray(Charsets.US_ASCII)
    val bundle:Path=base.resolve(hash(manifest))
    var payloadOpens=0
        private set
    private val resources=NativeResources { name -> when {
        name==DesktopMcpBundle.MANIFEST->manifest.inputStream()
        name.startsWith("mcp-server/")->byteArrayOf(1).inputStream().also{payloadOpens++}
        else->null
    } }

    fun prepare(progress:McpStartupProgress?=null):DesktopMcpBundle=DesktopMcpBundle.prepare(parent,resources,guard,progress)

    fun fill(count:Int) {
        if(!Files.exists(base))Files.createDirectory(base)
        repeat(count){index->
            val entry=Files.createDirectory(base.resolve("entry-$index"))
            Files.writeString(entry.resolve("preserve.txt"),"synthetic retained entry $index")
        }
    }

    fun inventory():Map<String,String> = Files.walk(base).use { paths ->
        paths.filter{it!=base}.toList().associate { path ->
            base.relativize(path).toString() to if(Files.isDirectory(path,LinkOption.NOFOLLOW_LINKS))"directory" else hash(Files.readAllBytes(path))
        }
    }

    override fun close(){Files.walk(parent).use{paths->paths.sorted(Comparator.reverseOrder()).forEach{Files.delete(it)}}}

    companion object {
        val names=listOf("vw-mcp-host.exe","vw-mcp-package.exe","node.exe","mcp/src/main.mjs",
            "vw-codex-host.exe","mcp/codex/main.mjs","mcp/codex/client.mjs","mcp/codex/package.mjs",
            "mcp/codex/process.mjs","mcp/codex/schema.mjs","mcp/codex/schema-worker.mjs","mcp/codex/image-profiles.json")
        private fun hash(bytes:ByteArray):String=MessageDigest.getInstance("SHA-256").digest(bytes)
            .joinToString(""){"%02x".format(it.toInt() and 255)}
    }
}

private class McpCapacityGuard:NativeFileGuard {
    var held=0
        private set
    override fun check(path:Path){
        val attributes=Files.readAttributes(path,BasicFileAttributes::class.java,LinkOption.NOFOLLOW_LINKS)
        check(!attributes.isSymbolicLink && !attributes.isOther)
    }
    override fun pin(path:Path):AutoCloseable {
        check(path);held++
        val closed=AtomicBoolean()
        return AutoCloseable{if(closed.compareAndSet(false,true))held--}
    }
    override fun pinDirectory(path:Path):AutoCloseable=pin(path)
}
