package com.visualworkbench.shared

import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.security.MessageDigest
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.joinAll

/** Closing fences new borrows, cancels current ones, and waits for their
 * cancellation cleanup before the native session can release the handle. The
 * close mutex must never be held while a receipt owner suspends. */
internal class NativeCompatibilityBorrowOwner {
    private val lock=Any()
    private var closed=false
    private val active=mutableSetOf<Job>()

    suspend fun <T> borrow(block:suspend()->T):T? {
        val job=currentCoroutineContext()[Job] ?: return null
        synchronized(lock) { if(closed)return null;active.add(job) }
        try { return block() } finally { synchronized(lock) { active.remove(job) } }
    }

    suspend fun closeAndJoin() {
        val borrowers=synchronized(lock) { closed=true;active.toList() }
        borrowers.forEach { it.cancel() }
        borrowers.joinAll()
    }
}

/** This interface belongs to the existing observed-receipt owner. Its callback
 * must retain that owner/current observation until completion, including cancel.
 * It must never reconstruct an owned receipt from caller JSON or a trusted flag. */
internal interface NativeCompatibilityObservationAccess {
    suspend fun withCurrentReceipt(
        expected:RemoteTargetState,
        receiptSha256:String,
        body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?,
    ):RemoteCompatibilityAdmission?
}

/** A projection supplied only by the actual joined observation owner. That
 * owner independently derives records from its effect/physical evidence; this
 * DTO does not turn probe/profile/Injected/ACK bytes into such evidence. */
internal class NativeOwnedCompatibilityReceipt(
    val target:RemoteTargetState,
    val toolId:String,
    val settingsDigest:String,
    receipt:ByteArray,
    records:List<RemoteToolCompatibility>,
) {
    val receipt:ByteArray=receipt.copyOf()
    val records:List<RemoteToolCompatibility> = records.toList()
}

internal object NativeRemoteCompatibility {
    suspend fun admitCurrent(
        expected:RemoteTargetState,bytes:ByteArray,
        current:()->RemoteTargetState?,access:NativeCompatibilityObservationAccess,
    ):RemoteCompatibilityAdmission? {
        fun currentOwner():Boolean=current()?.let{it.binding==expected.binding&&it.identity==expected.identity}==true
        if(!currentOwner())return null
        val result=admit(expected,bytes,access)
        kotlinx.coroutines.currentCoroutineContext().ensureActive()
        return result.takeIf{currentOwner()}
    }
    suspend fun admit(
        expected:RemoteTargetState,
        bytes:ByteArray,
        access:NativeCompatibilityObservationAccess,
    ):RemoteCompatibilityAdmission? {
        val requested=decode(expected,bytes)
        val digest=sha(bytes)
        return access.withCurrentReceipt(expected,digest){owned->
            if(owned==null||owned.target.binding!=expected.binding||owned.target.identity!=expected.identity||
                owned.toolId!=requested.toolId||owned.settingsDigest!=requested.settingsDigest||
                owned.receipt.size !in 1..REMOTE_COMPATIBILITY_IMPORT_LIMIT||
                sha(owned.receipt)!=digest||!owned.receipt.contentEquals(bytes)||owned.records!=requested.records)null
            else requested.takeIf{it.validFor(expected)}
        }
    }
    /** Strict bounded interchange projection. Syntax and hashes do not admit a
     * receipt: admit additionally requires exact owner-held bytes AND records. */
    internal fun decode(expected:RemoteTargetState,bytes:ByteArray):RemoteCompatibilityAdmission {
        require(bytes.size in 1..REMOTE_COMPATIBILITY_IMPORT_LIMIT)
        val text=Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
        val value=RemoteNativeJson.read(text)
        require(value.keys==setOf("schema","kind","identity","toolId","settingsDigest","observations"))
        require(RemoteNativeJson.uint(value["schema"])==1u&&value["kind"]=="paint-compatibility-observations")
        val image=RemoteNativeJson.obj(value["identity"])
        require(image.keys==setOf("editor","executableName","executableBlake3","executableBytes","fileVersion","packageFullName","packageVersion","shortcutProfileDigest"))
        require(image["editor"]=="Paint")
        val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,
            RemoteNativeJson.string(image["executableName"]),RemoteNativeJson.string(image["executableBlake3"]),
            RemoteNativeJson.ulong(image["executableBytes"]),RemoteNativeJson.optional(image["fileVersion"]),
            RemoteNativeJson.optional(image["packageFullName"]),RemoteNativeJson.optional(image["packageVersion"]),
            RemoteNativeJson.optional(image["shortcutProfileDigest"]))
        require(identity==expected.identity&&identity.valid())
        val tool=RemoteNativeJson.string(value["toolId"])
        val settings=RemoteNativeJson.string(value["settingsDigest"])
        val observations=RemoteNativeJson.array(value["observations"])
        require(observations.size in 1..RemoteCompatibilityAspect.entries.size)
        val records=observations.map{raw->
            val row=RemoteNativeJson.obj(raw)
            require(row.keys==setOf("aspect","result","method","inputApi","evidenceSha256","deviceModel"))
            RemoteToolCompatibility(identity,tool,settings,
                enumValueOf<RemoteEditorInputApi>(RemoteNativeJson.string(row["inputApi"])),
                enumValueOf<RemoteCompatibilityAspect>(RemoteNativeJson.string(row["aspect"])),
                enumValueOf<RemoteCompatibilityResult>(RemoteNativeJson.string(row["result"])),
                enumValueOf<RemoteCompatibilityMethod>(RemoteNativeJson.string(row["method"])),
                RemoteNativeJson.optional(row["evidenceSha256"]),RemoteNativeJson.optional(row["deviceModel"]))
        }
        return RemoteCompatibilityAdmission(expected,tool,settings,records).also{require(it.validFor(expected))}
    }
    internal fun sha(bytes:ByteArray):String=MessageDigest.getInstance("SHA-256").digest(bytes)
        .joinToString(""){"%02x".format(it.toInt() and 255)}
}
