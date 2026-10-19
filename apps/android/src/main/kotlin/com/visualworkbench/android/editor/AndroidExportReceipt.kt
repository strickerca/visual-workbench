package com.visualworkbench.android.editor

import com.visualworkbench.shared.FileExportResult
import com.visualworkbench.shared.WorkflowFailure
import com.visualworkbench.shared.WorkflowFailureKind
import java.math.BigDecimal
import java.nio.CharBuffer
import java.nio.charset.CodingErrorAction

/** Native completion is checked before any document-provider or share writer
 * opens. Top-level fields are distinct from nested image_output geometry. */
internal fun checkedAndroidExportReceipt(result: FileExportResult, intent: ExportCallerIntent) {
    val revision = result.revision
    if (revision.projectId != intent.binding.projectId || revision.hostSeq != intent.binding.hostSeq ||
        revision.stateHash != intent.binding.stateHash) throw WorkflowFailure(WorkflowFailureKind.Stale)
    if (result.encodedBytes !in 1uL..(512uL * 1024uL * 1024uL) ||
        !result.blake3.matches(Regex("[0-9a-f]{64}"))) throw RasterTransferFailure("The native export receipt has invalid encoded identity.")
    val fields = try {
        // UTF-8 never uses fewer bytes than UTF-16 code units for well-formed
        // text. Reject oversized input before the encoder allocates its buffer.
        if (result.metadataJson.length > 65_536) throw IllegalArgumentException("Oversized receipt")
        val bytes = Charsets.UTF_8.newEncoder().onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT).encode(CharBuffer.wrap(result.metadataJson))
        if (bytes.remaining() > 65_536) throw IllegalArgumentException("Oversized receipt")
        ReceiptJson(result.metadataJson).objectValue()
    } catch (_: Exception) { throw RasterTransferFailure("The native export receipt is malformed.") }
    fun dimension(name: String): UInt? = (fields[name] as? BigDecimal)?.takeIf { it.scale() == 0 }
        ?.let { runCatching { it.longValueExact() }.getOrNull() }
        ?.takeIf { it in 1..UInt.MAX_VALUE.toLong() }?.toUInt()
    if (fields["revision"] != intent.binding.stateHash || fields["source_asset"] != intent.geometry.sourceAssetId ||
        dimension("output_width") != intent.geometry.encodedWidth || dimension("output_height") != intent.geometry.encodedHeight)
        throw WorkflowFailure(WorkflowFailureKind.Stale)
}

/** Bounded JSON parser: duplicate keys, trailing values and malformed nested
 * extensions fail rather than selecting a convenient regex occurrence. */
private class ReceiptJson(private val source: String) {
    private var at = 0
    private var nodes = 0
    fun objectValue(): Map<String, Any?> {
        val result = value(0) as? Map<*, *> ?: error("Receipt is not an object")
        space(); require(at == source.length)
        return result.entries.associate { (key, item) -> key as String to item }
    }
    private fun space() { while (at < source.length && source[at] in " \r\n\t") at++ }
    private fun take(char: Char): Boolean = if (at < source.length && source[at] == char) { at++; true } else false
    private fun value(depth: Int): Any? {
        require(depth <= 16 && ++nodes <= 8192); space(); require(at < source.length)
        return when (source[at]) {
            '"' -> string()
            '{' -> {
                at++; space(); val result = linkedMapOf<String, Any?>()
                if (!take('}')) { do {
                    space(); val key = string(); require(key !in result); space(); require(take(':'))
                    result[key] = value(depth + 1); space()
                } while (take(',')); require(take('}')) }
                result
            }
            '[' -> {
                at++; space(); val result = ArrayList<Any?>()
                if (!take(']')) { do { result += value(depth + 1); space() } while (take(',')); require(take(']')) }
                result
            }
            't' -> literal("true", true)
            'f' -> literal("false", false)
            'n' -> literal("null", null)
            else -> {
                val start = at
                while (at < source.length && source[at] in "-+0123456789.eE") at++
                val raw = source.substring(start, at)
                require(raw.length in 1..128 && raw.matches(Regex("-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?")))
                BigDecimal(raw)
            }
        }
    }
    private fun literal(token: String, result: Any?): Any? {
        require(source.startsWith(token, at)); at += token.length; return result
    }
    private fun string(): String {
        require(take('"')); val out = StringBuilder()
        while (at < source.length) {
            val char = source[at++]
            if (char == '"') return out.toString()
            require(char >= ' ' && out.length <= 65_536)
            if (char != '\\') out.append(char) else {
                require(at < source.length)
                when (val escaped = source[at++]) {
                    '"', '\\', '/' -> out.append(escaped)
                    'b' -> out.append('\b'); 'f' -> out.append('\u000c')
                    'n' -> out.append('\n'); 'r' -> out.append('\r'); 't' -> out.append('\t')
                    'u' -> {
                        require(at + 4 <= source.length)
                        val digits = source.substring(at, at + 4)
                        require(digits.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' })
                        val code = digits.toInt(16)
                        out.append(code.toChar()); at += 4
                    }
                    else -> error("Invalid escape")
                }
            }
        }
        error("Incomplete string")
    }
}
