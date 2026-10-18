package com.visualworkbench.shared.assistedselection.backend

import java.io.RandomAccessFile
import kotlin.coroutines.CoroutineContext
import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.ensureActive

/** Minimal bounded TFLite v3 metadata reader. Random reads never load weights or
 * create a LiteRT model. All table/vector/string offsets are checked against the
 * already hashed local child; custom/Flex/delegate opcodes are rejected. */
internal fun inspectFlatbuffer(spec: ModelSpec, task: CoroutineContext = EmptyCoroutineContext) {
    RandomAccessFile(spec.file,"r").use { file ->
        val fb=Metadata(file,spec.bytes,task)
        if (fb.i32(4) != 0x334c4654) corrupt() // TFL3, little endian.
        val model=fb.i32(0).toLong(); if (fb.scalar(model,0,0) != 3) corrupt()
        val graphs=fb.vector(model,2,1); if (graphs.size != 1) corrupt()
        val graph=fb.indirect(graphs.single())
        val tensors=fb.vector(graph,0,32768).map(fb::indirect)
        fun check(specs: List<TensorSpec>, field: Int) {
            val indices=fb.vector(graph,field,8).map { fb.i32(it) }
            if (indices.toSet() != specs.map { it.index }.toSet() || indices.size != specs.size) corrupt()
            specs.forEach {
                val tensor=tensors.getOrNull(it.index) ?: corrupt()
                val type=fb.byteScalar(tensor,1,0)
                if (type != if (it.dtype == "float32") 0 else if (it.dtype == "int32") 2 else -1) corrupt()
                if (fb.stringField(tensor,3) != it.tensorName || fb.vector(tensor,0,4).map { p -> fb.i32(p) } != it.shape) corrupt()
            }
        }
        check(spec.inputs,1); check(spec.outputs,2)
        val codes=fb.vector(model,1,256).map {
            val table=fb.indirect(it)
            if (fb.field(table,1) != 0L) corrupt()
            val code=fb.scalar(table,3,0).takeIf { value -> value != 0 } ?: fb.byteScalar(table,0,0)
            val version=fb.scalar(table,2,1)
            if (code !in LITERT_BUILTINS || version !in 1..32) corrupt()
            code to version
        }
        if (codes.distinct().size != codes.size || codes.sortedWith(compareBy({it.first},{it.second})) != spec.operators) corrupt()
        val used=mutableSetOf<Int>()
        fb.vector(graph,3,8192).forEach {
            val operator=fb.indirect(it); val index=fb.scalar(operator,0,0)
            if (index !in codes.indices) corrupt()
            used+=index
        }
        if (used != codes.indices.toSet()) corrupt()
        val signatures=fb.vector(model,7,4).map(fb::indirect)
        val signature=signatures.singleOrNull { fb.stringField(it,2) == spec.signature } ?: corrupt()
        if (fb.scalar(signature,4,0) != 0) corrupt()
        fun signatureTensors(field: Int, specs: List<TensorSpec>) {
            val bindings=fb.vector(signature,field,8).map {
                val item=fb.indirect(it)
                fb.stringField(item,0) to fb.scalar(item,1,0)
            }
            if (bindings.size != specs.size || bindings.toSet() != specs.map { it.signatureName to it.index }.toSet()) corrupt()
        }
        signatureTensors(0,spec.inputs); signatureTensors(1,spec.outputs)
    }
}

private class Metadata(private val file: RandomAccessFile, private val size: Long, private val task: CoroutineContext) {
    private var reads=0
    private fun range(offset: Long, length: Long) {
        if (++reads > 2_000_000) limit()
        if (reads % 128 == 1) task.ensureActive()
        if (offset < 0 || length < 0 || offset > size-length) corrupt()
    }
    fun i32(offset: Long): Int { range(offset,4); file.seek(offset); return Integer.reverseBytes(file.readInt()) }
    private fun u16(offset: Long): Int { range(offset,2); file.seek(offset); return java.lang.Short.reverseBytes(file.readShort()).toInt() and 65535 }
    fun field(table: Long, index: Int): Long {
        range(table,4); val vtable=table-i32(table); val length=u16(vtable); val objectSize=u16(vtable+2)
        if (length !in 4..128 || length % 2 != 0 || objectSize !in 4..512) corrupt()
        range(vtable,length.toLong()); range(table,objectSize.toLong())
        val entry=4+index*2; if (entry+2 > length) return 0
        val offset=u16(vtable+entry); if (offset == 0) return 0
        if (offset !in 4 until objectSize) corrupt()
        return table+offset
    }
    fun scalar(table: Long, index: Int, fallback: Int): Int = field(table,index).let { if (it == 0L) fallback else i32(it) }
    fun byteScalar(table: Long, index: Int, fallback: Int): Int = field(table,index).let {
        if (it == 0L) fallback else { range(it,1); file.seek(it); file.readUnsignedByte() }
    }
    fun indirect(offset: Long): Long {
        val relative=i32(offset); if (relative <= 0) corrupt()
        return (offset+relative).also { range(it,4) }
    }
    fun vector(table: Long, index: Int, max: Int): List<Long> {
        val location=field(table,index); if (location == 0L) return emptyList()
        val base=indirect(location); val count=i32(base)
        if (count !in 0..max) corrupt(); range(base+4,count.toLong()*4)
        return List(count) { base+4+it*4L }
    }
    fun stringField(table: Long, index: Int): String {
        val location=field(table,index); if (location == 0L) corrupt()
        val base=indirect(location); val count=i32(base)
        if (count !in 1..256) corrupt(); range(base+4,count.toLong()+1)
        file.seek(base+4); val bytes=ByteArray(count); file.readFully(bytes)
        if (file.readUnsignedByte() != 0 || bytes.any { (it.toInt() and 255) !in 32..126 }) corrupt()
        return bytes.decodeToString()
    }
}

internal fun requireTensorContract(encoder: ModelSpec, decoder: ModelSpec) {
    fun expected(actual: List<TensorSpec>, specs: List<Triple<String,String,List<Int>>>) {
        if (actual.map { it.role } != specs.map { it.first } || actual.size != specs.size || actual.map { it.role }.distinct().size != actual.size ||
            actual.map { it.signatureName }.distinct().size != actual.size || actual.map { it.index }.distinct().size != actual.size) corrupt()
        specs.forEach { (name,type,shape) ->
            val item=actual.singleOrNull { it.role == name } ?: corrupt()
            if (item.dtype != type || item.shape != shape || item.signatureName != name || item.index < 0) corrupt()
        }
    }
    if (encoder.signature != "serving_default" || decoder.signature != "serving_default") corrupt()
    expected(encoder.inputs,listOf(Triple("image","float32",listOf(1,3,1024,1024))))
    expected(encoder.outputs,listOf(Triple("image_embeddings","float32",listOf(1,256,64,64))))
    expected(decoder.inputs,listOf(Triple("image_embeddings","float32",listOf(1,256,64,64)),
        Triple("point_coords","float32",listOf(1,66,2)),Triple("point_labels","int32",listOf(1,66)),
        Triple("point_valid","int32",listOf(1,66)),Triple("token_count","int32",listOf(1))))
    expected(decoder.outputs,listOf(Triple("mask_logits","float32",listOf(1,1,256,256)),Triple("score","float32",listOf(1,1))))
}

/** Finite reviewed builtin lowering, shared with the converter contract. */
internal val LITERT_BUILTINS: Map<Int,String> = mapOf(
    0 to "ADD",
    2 to "CONCATENATION",
    3 to "CONV_2D",
    4 to "DEPTHWISE_CONV_2D",
    6 to "DEQUANTIZE",
    9 to "FULLY_CONNECTED",
    14 to "LOGISTIC",
    17 to "MAX_POOL_2D",
    18 to "MUL",
    19 to "RELU",
    21 to "RELU6",
    22 to "RESHAPE",
    23 to "RESIZE_BILINEAR",
    25 to "SOFTMAX",
    28 to "TANH",
    34 to "PAD",
    36 to "GATHER",
    39 to "TRANSPOSE",
    40 to "MEAN",
    41 to "SUB",
    42 to "DIV",
    43 to "SQUEEZE",
    45 to "STRIDED_SLICE",
    47 to "EXP",
    49 to "SPLIT",
    53 to "CAST",
    55 to "MAXIMUM",
    57 to "MINIMUM",
    58 to "LESS",
    59 to "NEG",
    61 to "GREATER",
    62 to "GREATER_EQUAL",
    63 to "LESS_EQUAL",
    64 to "SELECT",
    65 to "SLICE",
    66 to "SIN",
    67 to "TRANSPOSE_CONV",
    69 to "TILE",
    70 to "EXPAND_DIMS",
    71 to "EQUAL",
    72 to "NOT_EQUAL",
    75 to "SQRT",
    76 to "RSQRT",
    77 to "SHAPE",
    78 to "POW",
    83 to "PACK",
    84 to "LOGICAL_OR",
    86 to "LOGICAL_AND",
    87 to "LOGICAL_NOT",
    88 to "UNPACK",
    94 to "FILL",
    96 to "RANGE",
    99 to "SQUARED_DIFFERENCE",
    102 to "SPLIT_V",
    108 to "COS",
    114 to "QUANTIZE",
    123 to "SELECT_V2",
    126 to "BATCH_MATMUL",
    130 to "BROADCAST_TO",
    150 to "GELU"
)

internal fun checkedOperator(code: Int, name: String, version: Int): Pair<Int,Int> {
    if (LITERT_BUILTINS[code] != name || version !in 1..32) corrupt()
    return code to version
}
