package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import java.nio.file.Path

/** Exercises the actual unapplied coordinator postimage with deterministic
 * coroutine dispatch. Native/catalog/connection IO use the existing interfaces. */
internal object T501RecapturePublicationProbe {
    private const val SESSION = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    private var passed = 0
    private fun uuid(n: Int) = "019a3aaa-0123-789a-8abc-${n.toString(16).padStart(12, '0')}"
    private fun info(n: Int) = PackageInfo(uuid(n), "generic", null,
        WorkflowBinding(uuid(100), uuid(101), 7uL, "a".repeat(64)), "b".repeat(64), "c".repeat(64),
        "2026-10-18T00:00:00.000Z", 128uL, 3u,
        listOf(PackageImageInfo("clean_source", "clean_source", "images/clean_source.png", 256u, 128u, 64uL)), true, false)
    private class Compiled(override val info: PackageInfo) : WorkbenchCompiledPackage {
        var closed = false
        override suspend fun close() { check(!closed); closed = true }
    }
    private class Catalog : WorkbenchPackageCatalog {
        val saved = linkedMapOf<String, PublishedPackage>()
        val publications = mutableListOf<String>()
        var beforePublish: suspend (PackageInfo) -> Unit = {}
        override suspend fun publish(packageValue: WorkbenchCompiledPackage): PublishedPackage {
            beforePublish(packageValue.info)
            val value = PublishedPackage(packageValue.info, "/synthetic/${packageValue.info.packageId}")
            saved[value.info.packageId] = value; publications += value.info.packageId
            return value
        }
        override suspend fun list() = saved.values.toList()
        override suspend fun lookup(packageId: String, target: String, manifestSha256: String): PublishedPackage =
            requireNotNull(saved[packageId]).also { require(it.info.target == target && it.info.manifestSha256 == manifestSha256) }
        override suspend fun readFile(packageId: String, target: String, manifestSha256: String, name: String, maxBytes: UInt): ByteArray = error("unused")
        override suspend fun pendingRetirement(): PackageRetirement? = null
        override suspend fun retire(packageId: String, target: String, manifestSha256: String): PackageRetirement {
            lookup(packageId, target, manifestSha256); saved.remove(packageId)
            return PackageRetirement(packageId, target, manifestSha256, true)
        }
        override suspend fun close() {}
    }
    private class Inbox : WorkbenchMcpInbox {
        override suspend fun submit(catalog: WorkbenchPackageCatalog, submission: McpInboxSubmission): McpInboxReceipt = error("unused")
        override suspend fun list(): List<McpInboxReceipt> = emptyList()
        override suspend fun readPng(receiptId: String, receiptBlake3: String, side: McpInboxSide): ByteArray = error("unused")
        override suspend fun pixels(receiptId: String, receiptBlake3: String, side: McpInboxSide, region: McpInboxRegion, assumeUntaggedSrgb: Boolean, allowDepthReduction: Boolean): McpInboxPixels = error("unused")
        override suspend fun retire(receiptId: String, receiptBlake3: String): McpInboxRetirement = error("unused")
        override suspend fun close() {}
    }
    private class Connection : McpConnection {
        override val agents = MutableStateFlow(listOf(McpAgent(SESSION, 42)))
        override val port = 12345
        val exposed = mutableListOf<String>()
        override suspend fun publish(directory: Path, manifestSha256: String) { exposed += directory.fileName.toString() }
        override suspend fun unpublish(packageId: String, target: String, manifestSha256: String) {}
        override suspend fun previewClaude(packageId: String, manifestSha256: String): McpClaudePreview = error("unused")
        override suspend fun pushClaude(connection: String, displayed: McpClaudePreview) = error("unused")
        override suspend fun grant(connection: String, selectors: List<String>, lifetimeMs: Int) {}
        override suspend fun revoke(connection: String) {}
        override suspend fun close() {}
    }
    private class Capture : McpCaptureEngine {
        var calls = 0; var blockedCall = 0
        val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
        val producerCancelled = CompletableDeferred<Unit>()
        val values = mutableListOf<Compiled>()
        override suspend fun select() = McpSelectedTarget("chosen", "Synthetic exact target")
        override suspend fun compile(selector: String, checkGrant: suspend () -> Unit): WorkbenchCompiledPackage {
            checkGrant(); require(selector == "chosen"); calls++
            val value = Compiled(info(calls)); values += value
            var watcher: Job? = null
            try {
                if (calls == blockedCall) {
                    val producer = requireNotNull(currentCoroutineContext()[Job])
                    watcher = CoroutineScope(currentCoroutineContext()).launch(start = CoroutineStart.UNDISPATCHED) {
                        try { awaitCancellation() }
                        finally { if (producer.isCancelled) producerCancelled.complete(Unit) }
                    }
                    withContext(NonCancellable) { entered.complete(Unit); release.await() }
                }
                currentCoroutineContext().ensureActive()
                return value
            } catch (error: Throwable) {
                // Match NativeMcpCaptureEngine's established producer contract:
                // a cancelled compile joins and releases its untransferred value.
                withContext(NonCancellable) { value.close() }
                throw error
            } finally {
                withContext(NonCancellable) { watcher?.cancelAndJoin() }
            }
        }
        override suspend fun close() { check(values.all { it.closed }) }
    }
    private class Fixture {
        val catalog = Catalog(); val connection = Connection(); val capture = Capture()
        val owner = DesktopMcpCoordinator(catalog, Inbox(), connection, capture) { 0L }
        suspend fun start(): PublishedPackage {
            owner.selectTarget(); owner.grant(SESSION, listOf("chosen"), 60_000)
            coroutineScope {
                val work = async(start = CoroutineStart.UNDISPATCHED) { owner.capture(SESSION, "chosen") }
                draw(); work.await()
            }
            return catalog.saved.values.single()
        }
        fun draw() { owner.indicatorDrawn(requireNotNull(owner.state.value.activeCapture).id) }
        suspend fun close() { capture.release.complete(Unit); owner.close() }
    }
    private suspend fun rejected(value: Deferred<*>) {
        var refused = false
        try { value.await() } catch (_: IllegalStateException) { refused = true }
        catch (_: IllegalArgumentException) { refused = true }
        check(refused) { "Retired original was accepted" }
    }
    private suspend fun test(name: String, body: suspend CoroutineScope.() -> Unit) {
        withTimeout(10_000) { supervisorScope { body() } }
        passed++; println("PASS $name")
    }

    @JvmStatic fun main(args: Array<String>) = runBlocking {
        // Retirement is definitely queued before recapture publication. All
        // controlled continuations run until suspension, with no sleep/polling.
        test("original retirement queued at catalogGate wins before durable recapture publication") {
            val f = Fixture(); val original = f.start()
            val held = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            val unrelated = Compiled(info(900))
            try {
                f.catalog.beforePublish = { if (it.packageId == unrelated.info.packageId) { held.complete(Unit); release.await() } }
                val holding = async(start = CoroutineStart.UNDISPATCHED) { f.owner.publish(unrelated) }; held.await()
                val retirement = async(start = CoroutineStart.UNDISPATCHED) { f.owner.retirePackage(original) }
                check(!retirement.isCompleted)
                val recapture = async(start = CoroutineStart.UNDISPATCHED) { f.owner.recaptureOriginal(SESSION, original.info) }
                f.draw(); check(f.capture.calls == 2 && !recapture.isCompleted)
                release.complete(Unit); holding.await(); retirement.await(); rejected(recapture)
                check(f.catalog.publications == listOf(info(1).packageId, unrelated.info.packageId)) {
                    "Retirement-winning recapture crossed catalog publication"
                }
                check(info(2).packageId !in f.connection.exposed && f.capture.values.all { it.closed })
            } finally { withContext(NonCancellable) { release.complete(Unit); unrelated.close(); f.close() } }
        }
        test("retirement during native compilation refuses publication and settles compiled owner") {
            val f = Fixture(); val original = f.start(); f.capture.blockedCall = 2
            try {
                val recapture = async(start = CoroutineStart.UNDISPATCHED) { f.owner.recaptureOriginal(SESSION, original.info) }
                f.draw(); f.capture.entered.await(); f.owner.retirePackage(original)
                f.capture.release.complete(Unit); rejected(recapture)
                check(f.catalog.publications == listOf(info(1).packageId) && f.connection.exposed.size == 1)
                check(f.capture.values.all { it.closed } && f.owner.state.value.activeCapture == null)
            } finally { withContext(NonCancellable) { f.close() } }
        }
        test("retirement while queued at captureGate prevents another native capture") {
            val f = Fixture(); val original = f.start(); f.capture.blockedCall = 2
            try {
                val ordinary = async(start = CoroutineStart.UNDISPATCHED) { f.owner.capture(SESSION, "chosen") }
                f.draw(); f.capture.entered.await()
                val recapture = async(start = CoroutineStart.UNDISPATCHED) { f.owner.recaptureOriginal(SESSION, original.info) }
                f.owner.retirePackage(original); f.capture.release.complete(Unit)
                ordinary.await(); rejected(recapture)
                check(f.capture.calls == 2 && f.catalog.publications == listOf(info(1).packageId, info(2).packageId))
                check(f.capture.values.all { it.closed })
            } finally { withContext(NonCancellable) { f.close() } }
        }
        test("cancellation waits for native settlement and never publishes its late result") {
            val f = Fixture(); val original = f.start(); f.capture.blockedCall = 2
            try {
                val recapture = async(start = CoroutineStart.UNDISPATCHED) { f.owner.recaptureOriginal(SESSION, original.info) }
                f.draw(); f.capture.entered.await(); recapture.cancel()
                // An outer caller's cancellation request can precede delivery
                // to the owned producer. Establish actual producer cancellation
                // before releasing its native result, without polling/sleeps.
                f.capture.producerCancelled.await()
                check(!recapture.isCompleted && !f.capture.values.last().closed)
                f.capture.release.complete(Unit); recapture.join()
                check(f.capture.values.all { it.closed }) { "Untransferred native value was not closed" }
                check(f.catalog.publications == listOf(info(1).packageId)) { "Cancelled recapture was published" }
                check(f.owner.state.value.activeCapture == null)
                val retry = async(start = CoroutineStart.UNDISPATCHED) { f.owner.recaptureOriginal(SESSION, original.info) }
                f.draw(); check(retry.await().recaptured.packageId == info(3).packageId)
            } finally { withContext(NonCancellable) { f.close() } }
        }
        test("publication that already owns catalogGate settles before later retirement") {
            val f = Fixture(); val original = f.start()
            val entered = CompletableDeferred<Unit>(); val release = CompletableDeferred<Unit>()
            try {
                f.catalog.beforePublish = { if (it.packageId == info(2).packageId) { entered.complete(Unit); release.await() } }
                val recapture = async(start = CoroutineStart.UNDISPATCHED) { f.owner.recaptureOriginal(SESSION, original.info) }
                f.draw(); entered.await()
                val retirement = async(start = CoroutineStart.UNDISPATCHED) { f.owner.retirePackage(original) }
                check(!retirement.isCompleted)
                release.complete(Unit); retirement.await()
                try { recapture.await() } catch (_: IllegalStateException) { /* final ticket check can lose after commit */ }
                check(info(2).packageId in f.catalog.publications && info(2).packageId in f.connection.exposed)
                check(info(2).packageId in f.catalog.saved && f.capture.values.all { it.closed })
            } finally { withContext(NonCancellable) { release.complete(Unit); f.close() } }
        }
        println("$passed actual coordinator-proposal race cases passed; native/device capture remains unrun.")
    }
}
