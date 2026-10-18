package com.visualworkbench.desktop.mcp

import com.visualworkbench.shared.*

/** Executable focused regressions; uses the unchanged production DTO declarations. */
internal object T501VerificationLeafProbe {
    private var passed = 0
    private fun test(name: String, body: () -> Unit) {
        body(); passed++; println("PASS $name")
    }
    private fun refuses(body: () -> Unit) {
        var refused = false
        try { body() } catch (_: IllegalArgumentException) { refused = true }
        catch (_: IllegalStateException) { refused = true }
        catch (_: java.nio.charset.CharacterCodingException) { refused = true }
        check(refused) { "Expected refusal" }
    }
    private fun uuid(n: Int) = "019a3aaa-0123-789a-8abc-${n.toString(16).padStart(12, '0')}"
    private fun info(n: Int = 1, markers: UInt = 3u) = PackageInfo(uuid(n), "generic", null,
        WorkflowBinding(uuid(100), uuid(101), 7uL, "a".repeat(64)), "b".repeat(64), "c".repeat(64),
        "2026-10-18T00:00:00.000Z", 128uL, markers,
        listOf(PackageImageInfo("clean_source", "clean_source", "images/clean_source.png", 256u, 128u, 64uL)), true, false)
    private fun receipt(original: PackageInfo = info()) = McpInboxReceipt(uuid(200), "d".repeat(64),
        original.packageId, original.target, original.manifestSha256, original.binding, original.sourceAssetId,
        "e".repeat(32), 1, null, "untrusted note",
        McpInboxImage(256u, 128u, 8u, 64uL, "f".repeat(64), null, 1u),
        McpInboxImage(256u, 128u, 8u, 64uL, "1".repeat(64), null, 1u), false)
    private fun bound(original: PackageInfo = info()): McpVerificationTargets = McpVerificationTargets().also {
        it.rememberCapture(original, "chosen-window"); check(it.rememberPackage(original))
    }
    private fun semanticJson(count: Int, name: String = "untrusted"): ByteArray {
        val source = info(); val snapshot = uuid(400)
        val rows = (0 until count).joinToString(",") { n ->
            McpJson.encode(linkedMapOf("eid" to "$snapshot/${n.toString().padStart(4, '0')}", "parent" to null,
                "name" to name, "role" to "button", "automation_id" to "stable$n", "resource_id" to null,
                "html_id" to null, "bounds_document" to listOf(10, 20, 30, 40), "bounds_clipped" to false,
                "text" to "untrusted", "enabled" to true, "focused" to false)).toString(Charsets.UTF_8)
        }
        return """{"schema_version":1,"project_id":"${source.binding.projectId}","document_id":"${source.binding.documentId}","snapshot_id":"$snapshot","platform":"uia","frame_delta_ms":17,"text_is_untrusted":true,"elements":[$rows]}""".toByteArray()
    }

    @JvmStatic fun main(args: Array<String>) {
        test("unknown source and unbound original cannot recapture") {
            val targets = McpVerificationTargets()
            check(!targets.rememberPackage(info())); refuses { targets.begin(info()) }
        }
        test("marked revisions reuse only the exact retained canonical source") {
            val original = info(); val targets = bound(original)
            val marked = original.copy(packageId = uuid(2), manifestSha256 = "2".repeat(64),
                binding = original.binding.copy(hostSeq = 8uL, stateHash = "3".repeat(64)))
            check(targets.rememberPackage(marked)); val ticket = targets.begin(marked)
            check(ticket.selector == "chosen-window"); targets.abandon(ticket)
            check(!targets.rememberPackage(marked.copy(packageId = uuid(3), sourceAssetId = "4".repeat(64))))
            check(!targets.rememberPackage(marked.copy(packageId = uuid(3), binding = marked.binding.copy(documentId = uuid(102)))))
            check(!targets.rememberPackage(marked.copy(packageId = uuid(3), binding = marked.binding.copy(projectId = uuid(103)))))
        }
        test("same pixels cannot replace the original window route") {
            val targets = bound()
            refuses { targets.rememberCapture(info(), "other-window") }
            val ticket = targets.begin(info()); check(ticket.selector == "chosen-window"); targets.abandon(ticket)
        }
        test("changed full revision hash and reused package identity refuse") {
            val original = info(); val targets = bound(original)
            refuses { targets.begin(original.copy(binding = original.binding.copy(stateHash = "8".repeat(64)))) }
            refuses { targets.rememberPackage(original.copy(manifestSha256 = "9".repeat(64))) }
        }
        test("retirement invalidates late completion without freeing active tickets") {
            val targets = bound(); val tickets = List(4) { targets.begin(info()) }
            targets.retire(info()); check(targets.rememberPackage(info()))
            refuses { targets.begin(info()) }
            refuses { targets.complete(tickets.first(), info(2)) }
            val fresh = targets.begin(info()); targets.abandon(fresh)
            tickets.drop(1).forEach(targets::abandon)
        }
        test("foreign consumed and same-package tickets cannot complete") {
            val first = bound(); val second = bound(); val ticket = first.begin(info())
            refuses { second.complete(ticket, info(2)) }
            val result = first.complete(ticket, info(2))
            check(result.original.packageId == info().packageId && result.recaptured.packageId == info(2).packageId)
            refuses { first.complete(ticket, info(3)) }
            val same = first.begin(info()); refuses { first.complete(same, info()) }
        }
        test("close invalidates tickets and forgets session routes") {
            val targets = bound(); val ticket = targets.begin(info()); targets.close(); targets.close()
            refuses { targets.complete(ticket, info(2)) }; refuses { targets.rememberCapture(info(), "chosen-window") }
            refuses { McpVerificationTargets().begin(info()) }
        }
        test("source and package admission remain bounded") {
            val targets = bound()
            for (n in 2..64) check(targets.rememberPackage(info(n)))
            refuses { targets.rememberPackage(info(65)) }
            val sources = McpVerificationTargets()
            for (n in 1..64) sources.rememberCapture(info(n).copy(binding = info().binding.copy(documentId = uuid(n + 500))), "chosen-window")
            refuses { sources.rememberCapture(info(65).copy(binding = info().binding.copy(documentId = uuid(600))), "chosen-window") }
        }
        test("review starts pending and cannot silently omit unreviewed markers") {
            val review = McpMarkerAssessments.open(info(), receipt()); val current = review.snapshot()
            check(current.markers.map { it.originalNumber } == listOf(1u, 2u, 3u))
            check(current.markers.all { it.verdict == McpMarkerVerdict.Pending })
            refuses { review.failures(current) }
        }
        test("failed subset keeps original numbers and exact inbox identity") {
            val review = McpMarkerAssessments.open(info(), receipt()); var current = review.snapshot()
            current = review.record(current, 1u, McpMarkerVerdict.Fail, "first")
            current = review.record(current, 2u, McpMarkerVerdict.Pass)
            current = review.record(current, 3u, McpMarkerVerdict.Fail, "third")
            val selection = requireNotNull(review.failures(current))
            check(selection.failed.map { it.originalNumber } == listOf(1u, 3u))
            check(selection.key.receiptBlake3 == receipt().receiptBlake3)
            check(selection.key.afterBlake3 == receipt().after!!.blake3)
            review.checkCurrent(selection)
            review.record(current, 3u, McpMarkerVerdict.Pass)
            refuses { review.checkCurrent(selection) }
        }
        test("stale and different-receipt owner events refuse") {
            val review = McpMarkerAssessments.open(info(), receipt()); val old = review.snapshot()
            val current = review.record(old, 1u, McpMarkerVerdict.Pass)
            refuses { review.record(old, 2u, McpMarkerVerdict.Fail) }
            refuses { review.record(current.copy(key = current.key.copy(receiptBlake3 = "9".repeat(64))), 2u, McpMarkerVerdict.Fail) }
            refuses { review.record(current, 0u, McpMarkerVerdict.Fail) }
            refuses { review.record(current, 4u, McpMarkerVerdict.Fail) }
        }
        test("all-pass review produces no empty follow-up") {
            val review = McpMarkerAssessments.open(info(), receipt()); var current = review.snapshot()
            for (n in 1u..3u) current = review.record(current, n, McpMarkerVerdict.Pass)
            check(review.failures(current) == null)
            current = review.record(current, 2u, McpMarkerVerdict.Pending)
            refuses { review.failures(current) }
        }
        test("receipt and source substitutions refuse before owner review") {
            val original = info(); val value = receipt()
            val changed = listOf(value.copy(retired = true), value.copy(manifestSha256 = "8".repeat(64)),
                value.copy(sourceAssetId = "8".repeat(64)), value.copy(target = "openai"),
                value.copy(packageId = uuid(8)), value.copy(binding = value.binding.copy(hostSeq = 8uL)),
                value.copy(binding = value.binding.copy(stateHash = "8".repeat(64))),
                value.copy(before = value.before.copy(width = 255u)), value.copy(after = null, text = "text return"))
            changed.forEach { wrong -> refuses { McpMarkerAssessments.open(original, wrong) } }
        }
        test("notes obey byte and total limits without changing failed review") {
            val original = info(markers = 5u); val review = McpMarkerAssessments.open(original, receipt(original)); var current = review.snapshot()
            for (n in 1u..4u) current = review.record(current, n, McpMarkerVerdict.Fail, "x".repeat(32768))
            refuses { review.record(current, 5u, McpMarkerVerdict.Fail, "x") }
            check(review.snapshot() == current)
            refuses { review.record(current, 5u, McpMarkerVerdict.Fail, "é".repeat(16385)) }
            refuses { review.record(current, 5u, McpMarkerVerdict.Fail, "\uD800") }
            refuses { review.record(current, 5u, McpMarkerVerdict.Fail, "a\u0000b") }
        }
        test("snapshots are immutable and close refuses late owner edits") {
            val review = McpMarkerAssessments.open(info(), receipt()); val current = review.snapshot()
            var immutable = false
            try { (current.markers as MutableList).clear() } catch (_: UnsupportedOperationException) { immutable = true }
            check(immutable); review.close(); review.close()
            refuses { review.record(current, 1u, McpMarkerVerdict.Pass) }; refuses { review.snapshot() }
        }
        val original = info()
        val recaptured = info(2).copy(binding = info().binding.copy(projectId = uuid(300), documentId = uuid(301)))
        val pair = McpVerificationRecapture(McpVerificationPackage.of(original), McpVerificationPackage.of(recaptured))
        val anchor = SemanticReference(SemanticPlatform.Uia, "${uuid(400)}/old", "untrusted", "button", "stable", null, null, Rect(10.0, 20.0, 30.0, 40.0))
        fun element(eid: String, rect: Rect, automation: String? = "stable") = SemanticElement(eid, null,
            "untrusted", "button", automation, null, null, rect, false, "untrusted", true, false)
        val before = McpAlignmentSnapshot(original.binding, uuid(400), SemanticPlatform.Uia,
            listOf(element(anchor.eid, anchor.boundsDocument)))
        val after = McpAlignmentSnapshot(recaptured.binding, uuid(401), SemanticPlatform.Uia,
            listOf(element("${uuid(401)}/new", Rect(15.0, 30.0, 30.0, 40.0))))
        val marker = Rect(12.0, 23.0, 4.0, 5.0)
        test("unique stable IDs align changed snapshot EIDs in source pixels") {
            val aligned = requireNotNull(alignMcpMarker(pair, 2u, marker, anchor, before, after))
            check(aligned.beforeBounds == marker && aligned.afterBounds == Rect(17.0, 33.0, 4.0, 5.0))
            check(aligned.originalNumber == 2u && aligned.originalEid != aligned.recapturedEid)
        }
        test("names text and local EIDs cannot supply a missing stable identity") {
            check(alignMcpMarker(pair, 2u, marker, anchor.copy(automationId = null), before, after) == null)
            check(alignMcpMarker(pair, 2u, marker, anchor, before, after.copy(platform = SemanticPlatform.AndroidAx)) == null)
        }
        test("duplicates in either snapshot and conflicting stable IDs stay unresolved") {
            check(alignMcpMarker(pair, 2u, marker, anchor, before.copy(elements = before.elements + before.elements[0].copy(eid = "${uuid(400)}/duplicate")), after) == null)
            check(alignMcpMarker(pair, 2u, marker, anchor, before, after.copy(elements = after.elements + after.elements[0].copy(eid = "${uuid(401)}/duplicate"))) == null)
            check(alignMcpMarker(pair, 2u, marker, anchor.copy(htmlId = "different"), before, after) == null)
        }
        test("clipping resizing and stale full snapshot bindings refuse alignment") {
            check(alignMcpMarker(pair, 2u, marker, anchor, before, after.copy(elements = listOf(after.elements[0].copy(boundsClipped = true)))) == null)
            check(alignMcpMarker(pair, 2u, marker, anchor, before, after.copy(elements = listOf(after.elements[0].copy(boundsDocument = Rect(15.0, 30.0, 31.0, 40.0))))) == null)
            refuses { alignMcpMarker(pair, 2u, marker, anchor, before, after.copy(binding = after.binding.copy(stateHash = "9".repeat(64)))) }
            refuses { alignMcpMarker(pair, 0u, marker, anchor, before, after) }
            refuses { alignMcpMarker(pair, 2u, marker.copy(x = Double.NaN), anchor, before, after) }
        }
        test("checked package semantics decode without inventing timing or target data") {
            val json = """{"schema_version":1,"project_id":"${original.binding.projectId}","document_id":"${original.binding.documentId}","snapshot_id":"${before.snapshotId}","platform":"uia","frame_delta_ms":17,"text_is_untrusted":true,"elements":[{"eid":"${anchor.eid}","parent":null,"name":"untrusted","role":"button","automation_id":"stable","resource_id":null,"html_id":null,"bounds_document":[10,20,30,40],"bounds_clipped":false,"text":"untrusted","enabled":true,"focused":false}]}""".toByteArray()
            val decoded = requireNotNull(McpAlignmentSnapshot.fromCheckedPackage(original, json))
            check(decoded == before)
            val invisible = json.toString(Charsets.UTF_8).replace("[10,20,30,40]", "[10,20,0,0]").toByteArray()
            check(McpAlignmentSnapshot.fromCheckedPackage(original, invisible)!!.elements.single().boundsDocument.width == 0.0)
            refuses { McpAlignmentSnapshot.fromCheckedPackage(original.copy(binding = original.binding.copy(documentId = uuid(800))), json) }
            val noSnapshot = """{"schema_version":1,"project_id":"${original.binding.projectId}","document_id":"${original.binding.documentId}","text_is_untrusted":true,"elements":[]}""".toByteArray()
            check(McpAlignmentSnapshot.fromCheckedPackage(original, noSnapshot) == null)
        }
        test("valid 2000 and maximum 4096 element snapshots retain all records") {
            for (count in listOf(2000, 4096)) {
                val json = semanticJson(count)
                // Preserve the deliberately smaller owner-IPC node allowance.
                refuses { McpJson.decode(json) }
                val decoded = requireNotNull(McpAlignmentSnapshot.fromCheckedPackage(original, json))
                check(decoded.elements.size == count)
                check(decoded.elements.last().automationId == "stable${count - 1}")
            }
        }
        test("semantic byte element and aggregate text ceilings refuse oversized input") {
            refuses { McpAlignmentSnapshot.fromCheckedPackage(original, semanticJson(4097)) }
            refuses { McpAlignmentSnapshot.fromCheckedPackage(original, ByteArray(4 * 1024 * 1024 + 1) { 32 }) }
            val tooMuchText = semanticJson(2000, "x".repeat(1100))
            check(tooMuchText.size < 4 * 1024 * 1024)
            refuses { McpAlignmentSnapshot.fromCheckedPackage(original, tooMuchText) }
        }
        test("semantic record framing preserves quoted delimiters and rejects malformed collections") {
            val quoted = "name has \\\"elements\\\":[{}], ] and }"
            check(McpAlignmentSnapshot.fromCheckedPackage(original, semanticJson(2, quoted))!!.elements.all { it.name == quoted })
            val text = semanticJson(2).toString(Charsets.UTF_8)
            for (bad in listOf(text.replace("},{", "}{"), text.dropLast(2) + ",]}",
                text.dropLast(1) + ",\"elements\":[]}", text.replace("\"role\":\"button\"", "\"role\":\"button\",\"role\":\"button\""),
                text.dropLast(8), text.replace("[10,20,30,40]", "[10,20,30,1e999]"),
                text.replace("\"name\":\"untrusted\"", "\"name\":\"\\uD800\""),
                text.replace("\"name\":\"untrusted\"", "\"name\":" + "[".repeat(34) + "0" + "]".repeat(34)))) {
                refuses { McpAlignmentSnapshot.fromCheckedPackage(original, bad.toByteArray()) }
            }
            val invalidUtf8 = semanticJson(1).also { it[it.indexOf('u'.code.toByte())] = 0xff.toByte() }
            refuses { McpAlignmentSnapshot.fromCheckedPackage(original, invalidUtf8) }
        }
        println("$passed verification leaf cases passed; no device/capture/alignment acceptance asserted.")
    }
}
