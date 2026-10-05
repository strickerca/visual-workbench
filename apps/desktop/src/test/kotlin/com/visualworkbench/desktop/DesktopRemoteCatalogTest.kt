package com.visualworkbench.desktop

import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.nio.file.*
import java.nio.file.attribute.BasicFileAttributes
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean

/** Synthetic package bytes and test-owned folders only; no native DLL, settings,
 * app-data, editor or executable profile authority is opened by these cases. */
class DesktopRemoteCatalogTest {
    @get:Rule val temporary=TemporaryFolder()
    @Test fun absentCatalogCreatesNoFilesAndNoPermissionDescriptor() {
        val guard=Guard();val root=folder()
        assertNull(DesktopRemoteCatalog.prepare(root,Resources(mutableMapOf()),guard))
        assertFalse(Files.exists(root));assertEquals(0,guard.live)
    }
    @Test fun exactPackagedCatalogRetainsNamespaceAndReusesOnlyTheHashBoundCache() {
        val files=resources();val guard=Guard();val first=requireNotNull(DesktopRemoteCatalog.prepare(folder(),files,guard))
        val key=Files.readAttributes(first.path,BasicFileAttributes::class.java).fileKey()
        val second=requireNotNull(DesktopRemoteCatalog.prepare(folder(),files,guard))
        try {
            assertEquals(first.path,second.path);assertEquals(key,Files.readAttributes(second.path,BasicFileAttributes::class.java).fileKey())
            assertTrue(guard.live>0);assertArrayEquals(files.data.getValue(DesktopRemoteCatalog.RESOURCE),Files.readAllBytes(first.path))
            assertEquals(hash(Files.readAllBytes(first.path)),first.sha256)
        }finally{second.close();first.close()}
        assertEquals(0,guard.live);assertTrue(Files.exists(first.path))
    }
    @Test fun missingPartnerDuplicateTraversalAndOversizedManifestsRefuseBeforeExtraction() {
        val good=resources();val manifest=good.data.getValue(DesktopRemoteCatalog.MANIFEST).toString(Charsets.US_ASCII)
        val invalid=listOf(
            mapOf(DesktopRemoteCatalog.MANIFEST to manifest.toByteArray()),
            mapOf(DesktopRemoteCatalog.RESOURCE to good.data.getValue(DesktopRemoteCatalog.RESOURCE)),
            good.data+mapOf(DesktopRemoteCatalog.MANIFEST to (manifest+manifest).toByteArray()),
            good.data+mapOf(DesktopRemoteCatalog.MANIFEST to manifest.replace("vw-remote-editor-catalog.json","../catalog.json").toByteArray()),
            good.data+mapOf(DesktopRemoteCatalog.MANIFEST to "x".repeat(257).toByteArray()),
            good.data+mapOf(DesktopRemoteCatalog.MANIFEST to manifest.replace(" ${good.data.getValue(DesktopRemoteCatalog.RESOURCE).size} "," 32769 ").toByteArray()),
        )
        for(data in invalid){val root=temporary.newFolder().toPath().resolve("not-created");refuses{DesktopRemoteCatalog.prepare(root,Resources(data.toMutableMap()),Guard())};assertFalse(Files.exists(root))}
    }
    @Test fun changedPackagedPayloadNeverPublishesAReadyCatalog() {
        for(payload in listOf(byteArrayOf(1),ByteArray(32769),"same-size wrong bytes".toByteArray())) {
            val data=resources();data.data[DesktopRemoteCatalog.RESOURCE]=payload;val root=temporary.newFolder().toPath().resolve("not-created")
            refuses{DesktopRemoteCatalog.prepare(root,data,Guard())};assertFalse(Files.exists(root))
        }
    }
    @Test fun changedExistingCatalogIsRefusedAndPreserved() {
        val files=resources();val first=requireNotNull(DesktopRemoteCatalog.prepare(folder(),files,Guard()));first.close()
        val changed="owner changed catalog bytes".toByteArray();Files.write(first.path,changed)
        refuses{DesktopRemoteCatalog.prepare(folder(),files,Guard())}
        assertArrayEquals(changed,Files.readAllBytes(first.path));assertTrue(Files.exists(first.path.parent.resolve("ready")))
    }
    @Test fun unknownCacheFilesAndChangedMarkersSurviveRefusal() {
        val files=resources();val first=requireNotNull(DesktopRemoteCatalog.prepare(folder(),files,Guard()));first.close()
        val unknown=first.path.parent.resolve("unrelated.txt");Files.writeString(unknown,"preserve")
        refuses{DesktopRemoteCatalog.prepare(folder(),files,Guard())};assertEquals("preserve",Files.readString(unknown))
        Files.delete(unknown);Files.writeString(first.path.parent.resolve("owner"),"changed marker")
        refuses{DesktopRemoteCatalog.prepare(folder(),files,Guard())};assertEquals("changed marker",Files.readString(first.path.parent.resolve("owner")))
    }
    @Test fun ancestorRefusalPrecedesCreatingAnyChildAndReleasesPriorLeases() {
        val parent=temporary.newFolder().toPath().toAbsolutePath().normalize();val root=parent.resolve("not-created/catalog")
        val guard=Guard{it==parent};refuses{DesktopRemoteCatalog.prepare(root,resources(),guard)}
        assertFalse(Files.exists(parent.resolve("not-created")));assertEquals(0,guard.live)
    }
    @Test fun injectedFinalFilePinFailureDoesNotOverwriteThePriorCatalog() {
        val files=resources();val first=requireNotNull(DesktopRemoteCatalog.prepare(folder(),files,Guard()));first.close()
        val guard=Guard{it==first.path};refuses{DesktopRemoteCatalog.prepare(folder(),files,guard)}
        assertEquals(0,guard.live);assertArrayEquals(files.data.getValue(DesktopRemoteCatalog.RESOURCE),Files.readAllBytes(first.path))
    }
    @Test fun catalogLeaseCloseFailureKeepsTheOwnerRetryable() {
        val guard=Guard();val owner=requireNotNull(DesktopRemoteCatalog.prepare(folder(),resources(),guard))
        guard.failClose=true
        refuses{owner.close()};assertTrue(guard.live>0)
        guard.failClose=false;owner.close();owner.close();assertEquals(0,guard.live)
    }
    @Test fun nativePendingRetirementKeepsCatalogAndNativeLeasesUntilTheSameOwnerRetries() {
        val nativeGuard=Guard();val runtime=DesktopNativeRuntime.prepareAt(temporary.root.toPath().resolve("native-runtime"),nativeResources(),nativeGuard)
        val catalogGuard=Guard();val assets=resources();val catalog=requireNotNull(runtime.packagedRemoteCatalog(assets,catalogGuard))
        var pending=true;runtime.armRetirementFence{if(pending)throw NativeRuntimeFailure("Actual remote retirement remains pending")}
        val nativeCount=nativeGuard.live;val catalogCount=catalogGuard.live
        try {
            refuses{runtime.close()};assertEquals(nativeCount,nativeGuard.live);assertEquals(catalogCount,catalogGuard.live)
            assertSame(catalog,runtime.packagedRemoteCatalog(assets,catalogGuard))
            pending=false;runtime.close();assertEquals(0,nativeGuard.live);assertEquals(0,catalogGuard.live)
        }finally{pending=false;runtime.close()}
    }
    @Test fun failedNewCacheAdmissionPreservesExactFilesWithoutPublishingAnOwner() {
        val files=resources();val root=folder();var reachedResourceGuard=false
        val guard=Guard{path->(path.fileName?.toString()==DesktopRemoteCatalog.RESOURCE).also{if(it)reachedResourceGuard=true}}
        refuses{DesktopRemoteCatalog.prepare(root,files,guard)}
        assertTrue("The resource guard must cause the refusal",reachedResourceGuard)
        assertEquals(0,guard.live)
        val manifest=files.data.getValue(DesktopRemoteCatalog.MANIFEST)
        val candidate=root.resolve(hash(manifest))
        assertTrue(Files.exists(candidate))
        assertArrayEquals(files.data.getValue(DesktopRemoteCatalog.RESOURCE),Files.readAllBytes(candidate.resolve(DesktopRemoteCatalog.RESOURCE)))
        assertTrue(Files.exists(candidate.resolve("owner")))
    }
    private fun folder():Path=temporary.root.toPath().toAbsolutePath().normalize().resolve("catalog-cache")
    private fun refuses(block:()->Unit){try{block();fail("Unverified catalog was accepted")}catch(_:NativeRuntimeFailure){}}
    private class Resources(val data:MutableMap<String,ByteArray>):NativeResources{override fun open(name:String)=data[name]?.let(::ByteArrayInputStream)}
    private fun resources():Resources {
        val payload="{\"schemaVersion\":4,\"entries\":[]}".toByteArray() // Cache fixture only; native Catalog rejects empty authority.
        return Resources(mutableMapOf(DesktopRemoteCatalog.RESOURCE to payload,DesktopRemoteCatalog.MANIFEST to "${hash(payload)} ${payload.size} ${DesktopRemoteCatalog.RESOURCE}\n".toByteArray(Charsets.US_ASCII)))
    }
    private fun nativeResources():Resources {
        val names=listOf("vw_core.dll","vw_host.dll","vw-connection-helper.exe","vw-capture-helper.exe","vw-hevc-helper.exe","vw-input-helper.exe")
        val data=names.associate{"win32-x86-64/$it" to "MZsynthetic".toByteArray()}.toMutableMap()
        data["vw-native-runtime.sha256"]=names.sorted().joinToString("\n",postfix="\n"){val b=data.getValue("win32-x86-64/$it");"${hash(b)} ${b.size} $it"}.toByteArray(Charsets.US_ASCII)
        return Resources(data)
    }
    private fun hash(bytes:ByteArray)=MessageDigest.getInstance("SHA-256").digest(bytes).joinToString(""){"%02x".format(it.toInt() and 255)}
    private class Guard(private val reject:(Path)->Boolean={false}):NativeFileGuard {
        var live=0;var failClose=false
        override fun check(path:Path){val info=Files.readAttributes(path,BasicFileAttributes::class.java,LinkOption.NOFOLLOW_LINKS);if(reject(path)||info.isSymbolicLink||info.isOther)throw NativeRuntimeFailure("Injected namespace refusal")}
        override fun pin(path:Path):AutoCloseable {check(path);live++;val closed=AtomicBoolean(false);return AutoCloseable{if(!closed.get()){if(failClose)throw NativeRuntimeFailure("Injected close failure");if(closed.compareAndSet(false,true))live--}}}
        override fun pinDirectory(path:Path)=pin(path)
    }
}
