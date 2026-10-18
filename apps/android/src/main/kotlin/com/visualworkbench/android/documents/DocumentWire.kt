package com.visualworkbench.android.documents

import com.visualworkbench.shared.*
import java.io.*
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

/** Private one-job payload. Hash/nonce/operation validation is still performed
 * by the native boundary before this payload becomes a shared DocumentReply. */
internal class DocumentWireResult(
    val sourceBlake3: String, val metadataJson: String, val bytes: ByteArray, val outputBlake3: String,
) {
    override fun toString(): String = "DocumentWireResult(bytes=${bytes.size})"
}

internal object DocumentWire {
    const val JOB = 1
    const val CANCEL = 2
    const val FINISHED = 3
    const val MAX_SOURCE = 64 * 1024 * 1024
    const val MAX_DATA = 128 * 1024 * 1024
    private const val MAX_METADATA = 1024 * 1024
    private val magic = byteArrayOf(86, 87, 68, 65, 80, 48, 49, 0)
    private val digest = Regex("[0-9a-f]{64}")
    private val failures = DocumentFailureKind.entries.filter { it != DocumentFailureKind.Stale }.associateBy { it.name }

    fun write(output: OutputStream, result: DocumentWireResult?, failure: DocumentFailureKind?) {
        if ((result == null) == (failure == null)) reject()
        if (failure == DocumentFailureKind.Stale) reject()
        val data = DataOutputStream(output)
        if (result != null) {
            validate(result)
            data.write(magic); data.writeInt(0)
            text(data, result.sourceBlake3, 64); text(data, result.outputBlake3, 64)
            text(data, result.metadataJson, MAX_METADATA)
            data.writeInt(result.bytes.size); data.write(result.bytes)
        } else {
            data.write(magic); data.writeInt(1); text(data, requireNotNull(failure).name, 64)
        }
        data.flush()
    }

    fun read(input: InputStream, maximumBytes: Int = MAX_DATA): DocumentWireResult {
        if (maximumBytes !in 0..MAX_DATA) throw DocumentFailure(DocumentFailureKind.Limit)
        try {
            val data = DataInputStream(input)
            val actual = ByteArray(magic.size); data.readFully(actual)
            if (!actual.contentEquals(magic)) reject()
            when (data.readInt()) {
                0 -> {
                    val source = text(data, 64); val hash = text(data, 64)
                    if (!digest.matches(source) || !digest.matches(hash)) reject()
                    val metadata = text(data, MAX_METADATA)
                    val length = data.readInt()
                    if (length !in 0..maximumBytes) throw DocumentFailure(DocumentFailureKind.Limit)
                    val bytes = ByteArray(length); data.readFully(bytes)
                    if (data.read() != -1) reject()
                    return DocumentWireResult(source, metadata, bytes, hash)
                }
                1 -> {
                    val failure = failures[text(data, 64)] ?: reject()
                    if (data.read() != -1) reject()
                    throw DocumentFailure(failure)
                }
                else -> reject()
            }
        } catch (failure: DocumentFailure) { throw failure }
        catch (_: IOException) { reject() }
    }

    fun source(input: InputStream, maximumBytes: Int = MAX_SOURCE): ByteArray {
        if (maximumBytes !in 1..MAX_SOURCE) throw DocumentFailure(DocumentFailureKind.Limit)
        val output = ByteArrayOutputStream(minOf(64 * 1024, maximumBytes))
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) break
            if (count == 0) reject()
            if (count > maximumBytes - output.size()) throw DocumentFailure(DocumentFailureKind.Limit)
            output.write(buffer, 0, count)
        }
        if (output.size() == 0) throw DocumentFailure(DocumentFailureKind.Invalid)
        return output.toByteArray()
    }

    private fun validate(result: DocumentWireResult) {
        if (!digest.matches(result.sourceBlake3) || !digest.matches(result.outputBlake3)) reject()
        if (result.bytes.size > MAX_DATA || result.metadataJson.length > MAX_METADATA || result.metadataJson.toByteArray(Charsets.UTF_8).size > MAX_METADATA)
            throw DocumentFailure(DocumentFailureKind.Limit)
    }
    private fun text(output: DataOutputStream, value: String, cap: Int) {
        val bytes = value.toByteArray(Charsets.UTF_8)
        if (bytes.size > cap) throw DocumentFailure(DocumentFailureKind.Limit)
        output.writeInt(bytes.size); output.write(bytes)
    }
    private fun text(input: DataInputStream, cap: Int): String {
        val length = input.readInt()
        if (length !in 0..cap) throw DocumentFailure(DocumentFailureKind.Limit)
        val bytes = ByteArray(length); input.readFully(bytes)
        return try {
            Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
        } catch (_: java.nio.charset.CharacterCodingException) { reject() }
    }
    private fun reject(): Nothing = throw DocumentFailure(DocumentFailureKind.Verification)
}
