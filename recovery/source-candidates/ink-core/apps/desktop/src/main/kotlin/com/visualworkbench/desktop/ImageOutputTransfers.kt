package com.visualworkbench.desktop

import com.visualworkbench.desktop.mcp.McpJson
import com.visualworkbench.shared.*
import java.math.BigDecimal

/** Every product export captures a committed frame. Missing metadata never
 * means an identity frame; loading/refusal is visible in the export dialog. */
internal fun captureImageOutputExport(state: EditorState, core: WorkbenchCore,
    output: ImageOutputSnapshot?): ExportRequest {
    val document=state.document ?: throw HandoffFailure("Open a document before exporting.")
    val snapshot=output?.takeIf{it.matches(document)}
        ?: throw HandoffFailure("Wait for committed output settings to load, or reopen Export to retry.")
    return captureExport(state,core,snapshot)
}

internal data class ImageOutputReceiptFields(val revision:String,val source:String,val width:UInt,val height:UInt)

/** Native metadata may contain another output_width inside image_output.
 * Interpret only typed top-level keys with the existing bounded JSON codec;
 * duplicate keys, malformed UTF-8 and nested spoofing remain refusals. */
internal fun imageOutputReceiptFields(text:String):ImageOutputReceiptFields {
    try {
        if(text.length>65_536)throw IllegalArgumentException()
        val bytes=text.toByteArray(Charsets.UTF_8)
        if(bytes.size>65_536)throw IllegalArgumentException()
        val value=McpJson.decode(bytes)
        fun string(key:String):String=(value[key] as? String)?.takeIf{it.length<=80} ?: throw IllegalArgumentException()
        fun integer(key:String):UInt {
            val number=(value[key] as? BigDecimal)?.longValueExact() ?: throw IllegalArgumentException()
            if(number !in 1..UInt.MAX_VALUE.toLong())throw IllegalArgumentException()
            return number.toUInt()
        }
        return ImageOutputReceiptFields(string("revision"),string("source_asset"),integer("output_width"),integer("output_height"))
    }catch(_:Exception){throw HandoffFailure("The completed export receipt is malformed or incomplete.")}
}
