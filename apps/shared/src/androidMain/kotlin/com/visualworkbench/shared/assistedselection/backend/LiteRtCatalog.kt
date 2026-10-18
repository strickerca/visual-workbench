package com.visualworkbench.shared.assistedselection.backend

import android.content.Context
import android.app.ActivityManager
import com.visualworkbench.shared.*
import kotlinx.coroutines.ensureActive
import kotlinx.serialization.json.*
import java.io.File
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.LinkOption
import java.security.MessageDigest
import java.util.zip.ZipFile
import kotlin.coroutines.CoroutineContext

/** Root-reviewed post-build APK identity, obtained from actual installed APKs.
 * This is separate from the upstream AAR identity because dex/native packaging
 * changes the executable artifact. These are CLOSED detached/installation-local
 * post-build review records injected by root, never an APK's embedded self-hash
 * or self-approval from hashing the current installation. No production identity
 * or duplicate review registry is supplied here. */
public data class LiteRtReviewedApk(public val name: String, public val bytes: Long, public val sha256: String)

/** A closed, source-bound allocation review. Native compiler/activation residency
 * includes unlinked immutable snapshots, mappings and compiled weight copies.
 * There is no public LiteRT allocator ceiling: its bound must be independently reviewed
 * for these exact model/runtime/APK pins before an entry is enabled. */
public data class LiteRtReviewedMemory(
    public val evidenceId: String, public val maximumSourcePixels: UInt,
    public val modelResidentBytes: ULong, public val activationBytes: ULong, public val runtimeBytes: ULong,
)

/** Immutable reviewed entry, never parsed from a model's self-asserted approval.
 * The five IDs include the actual closed redistribution/runtime decisions.
 * Both manifest paths are relative to the app-private catalog root. */
public data class LiteRtReviewedAdmission(
    public val pins: AssistedSelectionPins, public val modelManifest: String, public val runtimeManifest: String,
    public val installedApks: List<LiteRtReviewedApk>, public val memory: LiteRtReviewedMemory,
)

/** Created by trusted root registration after artifact/license/memory review.
 * An empty catalog is the only supplied production configuration. It deliberately
 * offers neither downloads nor an approval boolean or unchecked model bytes. */
public class LiteRtReviewedCatalog(public val root: File, admissions: List<LiteRtReviewedAdmission>) {
    internal val entries: List<LiteRtReviewedAdmission> = admissions.map { it.copy(installedApks=it.installedApks.toList()) }
    init {
        require(entries.size <= 8 && entries.map { it.pins }.distinct().size == entries.size)
    }
    public companion object {
        public fun empty(root: File): LiteRtReviewedCatalog = LiteRtReviewedCatalog(root,emptyList())
    }
}

internal data class TensorSpec(val role: String, val index: Int, val tensorName: String,
    val signatureName: String, val dtype: String, val shape: List<Int>) {
    val elements: Int get() = shape.fold(1) { a,b -> Math.multiplyExact(a,b) }
    val bytes: Int get() = Math.multiplyExact(elements,4)
}
internal data class ModelSpec(val file: File, val bytes: Long, val sha256: String, val signature: String,
    val inputs: List<TensorSpec>, val outputs: List<TensorSpec>, val operators: List<Pair<Int,Int>>)
internal data class VerifiedLiteRt(val entry: LiteRtReviewedAdmission, val encoder: ModelSpec, val decoder: ModelSpec,
    val workingSet: ULong, val gpuEligible: Boolean)

internal const val IMAGE_ELEMENTS: Int = 3*1024*1024
internal const val EMBEDDING_ELEMENTS: Int = 256*64*64
internal const val LOGIT_ELEMENTS: Int = 256*256
internal const val POOL_BYTES: ULong = 268435456uL
internal const val GRAPH_CONTRACT: String = "mobilesam-vit-t-fixed66-masked-v1"

internal fun unsupported(): Nothing = throw SelectionFailure(SelectionFailureKind.Unsupported)
internal fun corrupt(): Nothing = throw SelectionFailure(SelectionFailureKind.Corrupt)
internal fun limit(): Nothing = throw SelectionFailure(SelectionFailureKind.Limit)
internal fun digest(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
internal fun hashStream(stream: InputStream, expected: Long, context: CoroutineContext): String {
    if (expected !in 0..512L*1024*1024) limit()
    val hash=MessageDigest.getInstance("SHA-256"); val scratch=ByteArray(65536); var total=0L
    while (true) {
        context.ensureActive(); val read=stream.read(scratch); if (read < 0) break
        total+=read; if (total > expected) corrupt(); hash.update(scratch,0,read)
    }
    if (total != expected) corrupt()
    return hash.digest().joinToString("") { "%02x".format(it) }
}
internal fun hashFile(file: File, bytes: Long, context: CoroutineContext): String =
    file.inputStream().use { hashStream(it,bytes,context) }

/** Canonical byte equality also rejects duplicate keys, whitespace variants,
 * non-ASCII strings, floats/nulls and alternative escapes before any model load. */
internal fun canonicalManifest(bytes: ByteArray): JsonObject {
    if (bytes.size !in 3..131072 || bytes.any { (it.toInt() and 255) > 127 }) corrupt()
    var quoted=false; var escaped=false; var depth=0
    bytes.forEach { byte ->
        val ch=byte.toInt().toChar()
        if (quoted) {
            if (escaped) escaped=false else if (ch == '\\') escaped=true else if (ch == '"') quoted=false
        } else when (ch) {
            '"' -> quoted=true
            '{','[' -> { depth++; if (depth > 32) corrupt() }
            '}',']' -> { depth--; if (depth < 0) corrupt() }
        }
    }
    if (quoted || depth != 0) corrupt()
    val value=try { Json.parseToJsonElement(bytes.decodeToString()) } catch (_: Exception) { corrupt() }
    fun encode(item: JsonElement): String = when (item) {
        is JsonObject -> {
            if (item.size > 512 || item.keys.any { it.length > 256 || it.any { c -> c.code !in 32..126 } }) corrupt()
            item.entries.sortedBy { it.key }.joinToString(",","{","}") { JsonPrimitive(it.key).toString()+":"+encode(it.value) }
        }
        is JsonArray -> { if (item.size > 8192) corrupt(); item.joinToString(",","[","]") { encode(it) } }
        is JsonPrimitive -> if (item.isString) {
            if (item.content.length > 4096 || item.content.any { it.code !in 32..126 }) corrupt()
            JsonPrimitive(item.content).toString()
        } else {
            if (item.content != "true" && item.content != "false" &&
                (item.content.toLongOrNull() == null || item.content.toLong().toString() != item.content)) corrupt()
            item.content
        }
    }
    if (!(encode(value)+"\n").encodeToByteArray().contentEquals(bytes)) corrupt()
    return value as? JsonObject ?: corrupt()
}
internal fun JsonObject.text(key: String): String = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content ?: corrupt()
internal fun JsonElement.strictInteger(): Int = (this as? JsonPrimitive)?.takeIf { !it.isString }?.intOrNull ?: corrupt()
internal fun JsonObject.integer(key: String): Int = this[key]?.strictInteger() ?: corrupt()
internal fun JsonObject.longInteger(key: String): Long = (this[key] as? JsonPrimitive)?.takeIf { !it.isString }?.longOrNull ?: corrupt()
internal fun JsonObject.items(key: String, max: Int): List<JsonObject> =
    (this[key] as? JsonArray)?.takeIf { it.size <= max }?.map { it as? JsonObject ?: corrupt() } ?: corrupt()
internal fun privateFile(root: File, relative: String): File {
    if (relative.length !in 1..240 || relative.any { it.code !in 33..126 } || relative.contains('\\')) corrupt()
    val parts=relative.split('/'); if (parts.any { it.isEmpty() || it == "." || it == ".." || it.any { c -> !c.isLetterOrDigit() && c !in "_.-" } }) corrupt()
    val base=root.toPath().toAbsolutePath().normalize()
    if (base.toRealPath() != base || !Files.isDirectory(base,LinkOption.NOFOLLOW_LINKS)) corrupt()
    var current=base
    parts.forEach { part ->
        current=current.resolve(part)
        if (Files.isSymbolicLink(current) || current.toRealPath() != current) corrupt()
    }
    if (!Files.isRegularFile(current,LinkOption.NOFOLLOW_LINKS)) corrupt()
    return current.toFile()
}

internal suspend fun verifyCatalog(context: Context, catalog: LiteRtReviewedCatalog, pins: AssistedSelectionPins,
    task: CoroutineContext): VerifiedLiteRt {
    pins.admitShape()
    val entry=catalog.entries.singleOrNull { it.pins == pins } ?: unsupported()
    val root=catalog.root.toPath().toAbsolutePath().normalize()
    val privateRoots=listOf(context.filesDir,context.noBackupFilesDir).map { it.toPath().toRealPath() }
    if (privateRoots.none { root.startsWith(it) && root != it } || root.toRealPath() != root) corrupt()
    fun manifest(path: String, hash: String): JsonObject {
        val file=privateFile(catalog.root,path)
        if (file.length() !in 3..131072) limit()
        val scratch=ByteArray(131073); var count=0
        file.inputStream().use { input ->
            while (count < scratch.size) {
                task.ensureActive(); val read=input.read(scratch,count,scratch.size-count)
                if (read < 0) break
                count+=read
            }
        }
        if (count !in 3..131072) corrupt()
        val bytes=scratch.copyOf(count); if (digest(bytes) != hash) corrupt()
        return canonicalManifest(bytes)
    }
    val model=manifest(entry.modelManifest,pins.modelArtifactSha256)
    val runtime=manifest(entry.runtimeManifest,pins.runtimeArtifactSha256)
    if (model.text("schema") != "vw-assisted-model-v1" || model.text("model") != "MobileSAM-vit_t" ||
        model.text("graph_contract") != GRAPH_CONTRACT ||
        (model["preprocessing"] as? JsonObject)?.text("schema") != "sam-rgb-resize-longest1024-v1" ||
        (model["postprocessing"] as? JsonObject)?.text("schema") != "sam-mask0-logits-inverse-v1" ||
        runtime.text("schema") != "vw-assisted-runtime-v1" || runtime.text("runtime") != "LiteRT" || runtime.text("version") != "2.3.0") corrupt()
    val checkpoint=model["checkpoint"] as? JsonObject ?: corrupt()
    if (model.text("source_commit") != "f706ad9c4eb7f219c00d9050e46328518ffb65d2" ||
        checkpoint.longInteger("bytes") != 40728226L ||
        checkpoint.text("sha256") != "6dbb90523a35330fedd7f1d3dfc66f995213d81b29a5ca8108dbcdd4e37d6c2f") corrupt()
    val converter=model["converter"] as? JsonObject ?: corrupt()
    val converterFiles=converter.items("files",8)
    if (converterFiles.isEmpty()) corrupt()
    converterFiles.forEach {
        val file=privateFile(catalog.root,it.text("path"))
        val size=it.longInteger("bytes")
        if (size !in 1..1048576 || file.length() != size || hashFile(file,size,task) != it.text("sha256")) corrupt()
    }
    val artifacts=model.items("artifacts",2); if (artifacts.size != 2) corrupt()
    fun parse(spec: JsonObject): ModelSpec {
        val file=privateFile(catalog.root,spec.text("path")); val size=spec.longInteger("bytes")
        if (size !in 16..128L*1024*1024 || file.length() != size || hashFile(file,size,task) != spec.text("sha256")) corrupt()
        fun tensors(key: String): List<TensorSpec> = spec.items(key,8).map {
            val shape=(it["shape"] as? JsonArray)?.map { n -> n.strictInteger() } ?: corrupt()
            if (shape.size !in 1..4 || shape.any { n -> n <= 0 }) corrupt()
            TensorSpec(it.text("name"),it.integer("tensor_index"),it.text("tensor_name"),it.text("signature_name"),it.text("dtype"),shape)
        }
        val operators=spec.items("operators",256).map { checkedOperator(it.integer("code"),it.text("name"),it.integer("version")) }
        val result=ModelSpec(file,size,spec.text("sha256"),spec.text("signature_key"),tensors("inputs"),tensors("outputs"),operators)
        inspectFlatbuffer(result,task)
        return result
    }
    val encoder=parse(artifacts.singleOrNull { it.text("role") == "encoder" } ?: corrupt())
    val decoder=parse(artifacts.singleOrNull { it.text("role") == "prompt_decoder" } ?: corrupt())
    requireTensorContract(encoder,decoder)
    val expectedRuntime=mapOf(
        "litert-api" to (945619L to "20f630e9ed0b4f0564da616fd3d6406f3013cacad7d492cbe2963330f519256b"),
        "litert" to (7640140L to "9508c6e9142646f0f2bc1fe68b3925fb470ee7ddd29114540339a04a8ab189dc"),
        "litert-gpu" to (4695499L to "173a4c3813601f7e9542f4e6a49e237775e2f9ce1e0948f35407098bffb1824f"))
    val runtimeArtifacts=runtime.items("artifacts",3)
    if (runtimeArtifacts.map { it.text("role") }.toSet() != expectedRuntime.keys || runtimeArtifacts.size != 3) corrupt()
    val native=mutableMapOf<String,Pair<Long,String>>()
    runtimeArtifacts.forEach {
        val (size,sha)=expectedRuntime.getValue(it.text("role")); val file=privateFile(catalog.root,it.text("path"))
        if (it.longInteger("bytes") != size || it.text("sha256") != sha || hashFile(file,size,task) != sha) corrupt()
        ZipFile(file).use { zip ->
            val names=zip.entries().asSequence().filter { e -> e.name.startsWith("jni/arm64-v8a/") && e.name.endsWith(".so") }.toList()
            if (names.size != 1) corrupt()
            names.forEach { child ->
                if (child.size !in 1..64L*1024*1024) corrupt()
                val name=child.name.substringAfterLast('/')
                if (native.put(name,child.size to zip.getInputStream(child).use { input -> hashStream(input,child.size,task) }) != null) corrupt()
            }
        }
    }
    verifyInstalled(context,entry.installedApks,native,task)
    val memory=entry.memory
    if (memory.evidenceId.length !in 1..128 || memory.maximumSourcePixels == 0u || memory.maximumSourcePixels > 50_000_000u ||
        memory.modelResidentBytes < 2uL*(encoder.bytes+decoder.bytes).toULong() || memory.activationBytes == 0uL || memory.runtimeBytes == 0uL) unsupported()
    // Covers native tensor padding/copies, Java float arrays, Pillow row buffers,
    // source lease foreign pixels and full-D output. Native activation/compiled
    // residency is separately pinned by the mandatory closed memory review.
    val buffers=4uL*IMAGE_ELEMENTS.toULong()*4uL+6uL*EMBEDDING_ELEMENTS.toULong()*4uL+
        2uL*LOGIT_ELEMENTS.toULong()*4uL+8uL*1024uL*1024uL+memory.maximumSourcePixels.toULong()*5uL+262144uL
    val terms=listOf(buffers,memory.modelResidentBytes,memory.activationBytes,memory.runtimeBytes)
    var working=0uL; terms.forEach { if (it > POOL_BYTES || working > POOL_BYTES-it) unsupported(); working+=it }
    val gles=(context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager)?.deviceConfigurationInfo?.reqGlEsVersion ?: 0
    return VerifiedLiteRt(entry,encoder,decoder,working,gles >= 0x30000)
}

private fun verifyInstalled(context: Context, pins: List<LiteRtReviewedApk>, native: Map<String,Pair<Long,String>>, task: CoroutineContext) {
    val info=context.applicationInfo
    val files=(listOf(info.sourceDir)+info.splitSourceDirs.orEmpty().toList()).map(::File)
    if (files.size !in 1..16 || pins.size != files.size || pins.map { it.name }.distinct().size != pins.size) corrupt()
    val seen=mutableSetOf<String>()
    files.forEach { file ->
        val pin=pins.singleOrNull { it.name == file.name } ?: unsupported()
        if (Files.isSymbolicLink(file.toPath()) || file.toPath().toRealPath() != file.toPath().toAbsolutePath().normalize() ||
            file.length() != pin.bytes || hashFile(file,pin.bytes,task) != pin.sha256) unsupported()
        ZipFile(file).use { zip ->
            val entries=zip.entries(); var count=0
            while (entries.hasMoreElements()) {
                if (++count > 200000) limit()
                val item=entries.nextElement()
                if (item.name.startsWith("lib/arm64-v8a/")) native[item.name.substringAfterLast('/')]?.let { (size,sha) ->
                    if (!seen.add(item.name.substringAfterLast('/')) || item.size != size ||
                        zip.getInputStream(item).use { hashStream(it,size,task) } != sha) corrupt()
                }
            }
        }
    }
    if (seen != native.keys) unsupported()
    native.forEach { (name,expected) ->
        val extracted=File(info.nativeLibraryDir,name)
        if (extracted.exists() && (Files.isSymbolicLink(extracted.toPath()) ||
            hashFile(extracted,expected.first,task) != expected.second)) corrupt()
    }
}
