package com.visualworkbench.desktop.mcp

import java.math.BigDecimal
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction

/** Deliberately narrow owner-IPC JSON codec. No reflection, polymorphic tags or
 * dependency on an app-global serializer. Duplicate keys and amplification fail. */
internal object McpJson {
    const val MAX = 17 * 1024 * 1024
    fun decode(bytes: ByteArray): Map<String, Any?> {
        require(bytes.size <= MAX)
        val text = Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
        val parser = Parser(text)
        val result = parser.value(0)
        parser.space(); require(parser.at == text.length)
        @Suppress("UNCHECKED_CAST")
        return result as? Map<String, Any?> ?: error("MCP frame must be an object")
    }
    fun encode(value: Map<String, Any?>): ByteArray {
        val out = StringBuilder()
        var nodes = 0
        fun append(s: String) { require(out.length.toLong() + s.length <= MAX); out.append(s) }
        fun string(s: String) {
            append("\"")
            s.forEach { c -> when(c) { '\\' -> append("\\\\"); '"' -> append("\\\""); else -> if(c < ' ') append("\\u%04x".format(c.code)) else append(c.toString()) } }
            append("\"")
        }
        fun write(v: Any?, depth: Int) {
            require(depth <= 32 && ++nodes <= 32768)
            when(v) {
                null -> append("null")
                is String -> string(v)
                is Boolean -> append(v.toString())
                is Int, is Long, is BigDecimal -> append(v.toString())
                is Map<*, *> -> { append("{"); v.entries.forEachIndexed { i,e -> if(i>0) append(","); string(e.key as String); append(":"); write(e.value,depth+1) }; append("}") }
                is List<*> -> { append("["); v.forEachIndexed { i,e -> if(i>0) append(","); write(e,depth+1) }; append("]") }
                else -> error("Unsupported MCP owner value")
            }
        }
        write(value,0)
        val buffer = Charsets.UTF_8.newEncoder().onMalformedInput(CodingErrorAction.REPORT).encode(java.nio.CharBuffer.wrap(out))
        require(buffer.remaining() <= MAX)
        return ByteArray(buffer.remaining()).also { buffer.get(it) }
    }
    private class Parser(val text: String) {
        var at = 0; var nodes = 0
        fun space() { while(at < text.length && text[at] in " \r\n\t") at++ }
        fun value(depth: Int): Any? {
            require(depth <= 32 && ++nodes <= 32768); space(); require(at < text.length)
            return when(text[at]) {
                '"' -> string()
                '{' -> { at++; val out = linkedMapOf<String, Any?>(); space(); if(take('}')) out else {
                    do { space(); require(at < text.length && text[at]=='"'); val key=string(); require(!out.containsKey(key)); space(); require(take(':')); out[key]=value(depth+1); space() } while(take(',')); require(take('}')); out } }
                '[' -> { at++; val out=mutableListOf<Any?>(); space(); if(take(']')) out else { do {out.add(value(depth+1));space()} while(take(','));require(take(']'));out } }
                't' -> literal("true",true)
                'f' -> literal("false",false)
                'n' -> literal("null",null)
                else -> { val start=at; while(at<text.length && text[at] in "-+0123456789.eE") at++; val raw=text.substring(start,at); require(raw.length in 1..128 && raw.matches(Regex("-?(0|[1-9][0-9]*)(\\.[0-9]+)?([eE][+-]?[0-9]+)?"))); BigDecimal(raw) }
            }
        }
        fun take(c: Char): Boolean = if(at<text.length && text[at]==c) {at++;true}else false
        fun literal(raw:String,result:Any?):Any? {require(text.startsWith(raw,at));at+=raw.length;return result}
        fun string(): String {
            require(take('"')); val out=StringBuilder()
            while(at<text.length) {
                val c=text[at++]; if(c=='"') return out.toString()
                require(c >= ' ')
                if(c!='\\') out.append(c) else {
                    require(at<text.length)
                    when(val escape=text[at++]) {
                        '"','\\','/' -> out.append(escape)
                        'b' -> out.append('\b'); 'f' -> out.append('\u000c'); 'n' -> out.append('\n'); 'r' -> out.append('\r'); 't' -> out.append('\t')
                        'u' -> {require(at+4<=text.length);out.append(text.substring(at,at+4).toInt(16).toChar());at+=4}
                        else -> error("Invalid MCP JSON escape")
                    }
                }
            };error("Incomplete MCP JSON string")
        }
    }
}
