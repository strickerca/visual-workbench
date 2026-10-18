package com.visualworkbench.shared.assistedselection.backend

import com.visualworkbench.shared.*
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.*
import org.junit.Test
import java.io.File
import java.nio.file.Files

class LiteRtCatalogTest {
    @Test fun canonicalManifestsRefuseDuplicatesWhitespaceAliasesNonAsciiFloatsNullAndDeepInput() {
        val valid=canonicalManifest("{\"a\":1,\"b\":\"file.tflite\"}\n".encodeToByteArray())
        assertEquals(1,valid.integer("a")); assertEquals("file.tflite",valid.text("b"))
        val invalid=listOf("{\"a\":1,\"a\":1}\n","{\"a\":1 }\n","{\"b\":1,\"a\":2}\n",
            "{\"a\":1.0}\n","{\"a\":null}\n","{\"a\":\"é\"}\n","{\"a\":\"\\u0061\"}\n",
            "[".repeat(33)+"0"+"]".repeat(33)+"\n")
        invalid.forEach { refuses { canonicalManifest(it.encodeToByteArray()) } }
        refuses { canonicalManifest(ByteArray(131073) { 32 }) }
    }

    @Test fun quotedNumbersAreNeverAcceptedAsIntegersOrTensorDimensions() {
        val quoted=canonicalManifest("{\"bytes\":\"10\",\"index\":\"1\"}\n".encodeToByteArray())
        refuses { quoted.integer("index") }; refuses { quoted.longInteger("bytes") }
        refuses { JsonPrimitive("1024").strictInteger() }
        assertEquals(1024,JsonPrimitive(1024).strictInteger())
        refuses { JsonPrimitive(true).strictInteger() }
    }

    @Test fun relativeArtifactPathsRefuseAliasesTraversalAndSymbolicLinks() {
        val directory=Files.createTempDirectory("vw-backend-catalog-test").toFile()
        try {
            val real=File(directory,"model.tflite").also { it.writeBytes(byteArrayOf(1)) }
            assertEquals(real,privateFile(directory,"model.tflite"))
            listOf("../model.tflite","./model.tflite","a/../model.tflite","a//model.tflite","/model.tflite",
                "model.tflite/","a\\model.tflite","a:model.tflite").forEach { refuses { privateFile(directory,it) } }
            val link=File(directory,"alias.tflite"); Files.createSymbolicLink(link.toPath(),real.toPath())
            refuses { privateFile(directory,"alias.tflite") }
        } finally { directory.deleteRecursively() }
    }

    @Test fun graphContractRequiresExactRolesOrderSignatureTypesAndDimensions() {
        val encoder=encoder(); val decoder=decoder(); requireTensorContract(encoder,decoder)
        val changed=listOf(decoder.copy(inputs=decoder.inputs.reversed()),
            decoder.copy(inputs=decoder.inputs.map { if (it.role == "point_labels") it.copy(dtype="float32") else it }),
            decoder.copy(inputs=decoder.inputs.map { if (it.role == "point_valid") it.copy(signatureName="point_labels") else it }),
            decoder.copy(inputs=decoder.inputs.map { if (it.role == "point_coords") it.copy(shape=listOf(1,65,2)) else it }),
            decoder.copy(signature="other"),decoder.copy(outputs=decoder.outputs.map { if (it.role == "mask_logits") it.copy(shape=listOf(1,3,256,256)) else it }))
        changed.forEach { refuses { requireTensorContract(encoder,it) } }
        refuses { requireTensorContract(encoder.copy(inputs=encoder.inputs+encoder.inputs),decoder) }
    }

    @Test fun finiteBuiltinCatalogRefusesCustomFlexDelegateUnknownNamesAndVersions() {
        assertEquals(0 to 1,checkedOperator(0,"ADD",1))
        assertEquals(126 to 2,checkedOperator(126,"BATCH_MATMUL",2))
        listOf(Triple(32,"CUSTOM",1),Triple(51,"DELEGATE",1),Triple(999,"FUTURE",1),
            Triple(0,"FlexAdd",1),Triple(0,"ADD",0),Triple(0,"ADD",33),Triple(0,"ADD",65)).forEach {
            refuses { checkedOperator(it.first,it.second,it.third) }
        }
    }

    @Test fun metadataReadsValidateActualTensorNameShapeSignatureAndBoundsWithoutLoadingModels() {
        val file=Files.createTempFile("vw-tflite-metadata-test",".tflite").toFile()
        try {
            val valid=metadataFixture(); file.writeBytes(valid)
            val spec=encoder().copy(file=file,bytes=valid.size.toLong())
            inspectFlatbuffer(spec)
            refuses { inspectFlatbuffer(spec.copy(inputs=spec.inputs.map { it.copy(tensorName="different") })) }
            refuses { inspectFlatbuffer(spec.copy(inputs=spec.inputs.map { it.copy(shape=listOf(1,3,512,512)) })) }
            refuses { inspectFlatbuffer(spec.copy(signature="missing")) }
            refuses { inspectFlatbuffer(spec.copy(operators=listOf(0 to 1))) }
            file.writeBytes(valid.copyOf(12)); refuses { inspectFlatbuffer(spec.copy(bytes=12)) }
            val corrupt=valid.copyOf(); repeat(4) { corrupt[it]=127 }; file.writeBytes(corrupt)
            refuses { inspectFlatbuffer(spec) }
        } finally { file.delete() }
    }

    private fun refuses(block: () -> Unit) {
        try { block(); fail("Untrusted metadata admitted") } catch (_: SelectionFailure) { }
    }
    private fun tensor(name: String, index: Int, type: String, vararg shape: Int): TensorSpec =
        TensorSpec(name,index,name,name,type,shape.toList())
    private fun encoder(): ModelSpec = ModelSpec(File("unused"),1,"","serving_default",
        listOf(tensor("image",0,"float32",1,3,1024,1024)),listOf(tensor("image_embeddings",1,"float32",1,256,64,64)),emptyList())
    private fun decoder(): ModelSpec = ModelSpec(File("unused"),1,"","serving_default",listOf(
        tensor("image_embeddings",0,"float32",1,256,64,64),tensor("point_coords",1,"float32",1,66,2),
        tensor("point_labels",2,"int32",1,66),tensor("point_valid",3,"int32",1,66),tensor("token_count",4,"int32",1)),
        listOf(tensor("mask_logits",5,"float32",1,1,256,256),tensor("score",6,"float32",1,1)),emptyList())

    /** Structural metadata fixture only, no weights/executable inference graph.
     * Tiny forward-offset builder avoids importing a model/runtime test engine. */
    private fun metadataFixture(): ByteArray {
        val b=FixtureBuffer(); val model=b.table(8,setOf(0,1,2,7)); b.int(0,model); b.int(4,0x334c4654)
        b.int(model+4,3); b.ref(model+8,b.vector(0))
        val graphs=b.vector(1); b.ref(model+12,graphs)
        val graph=b.table(4,setOf(0,1,2,3)); b.ref(graphs+4,graph)
        val tensors=b.vector(2); b.ref(graph+4,tensors)
        encoder().let { spec -> (spec.inputs+spec.outputs).forEachIndexed { index,tensor ->
            val table=b.table(4,setOf(0,1,3)); b.ref(tensors+4+index*4,table)
            val shape=b.vector(tensor.shape.size); b.ref(table+4,shape)
            tensor.shape.forEachIndexed { i,n -> b.int(shape+4+i*4,n) }
            b.int(table+8,0); b.ref(table+16,b.string(tensor.tensorName))
        } }
        val inputs=b.vector(1); b.int(inputs+4,0); b.ref(graph+8,inputs)
        val outputs=b.vector(1); b.int(outputs+4,1); b.ref(graph+12,outputs)
        b.ref(graph+16,b.vector(0))
        val signatures=b.vector(1); b.ref(model+32,signatures)
        val signature=b.table(5,setOf(0,1,2)); b.ref(signatures+4,signature)
        for ((field,tensor) in listOf(0 to encoder().inputs.single(),1 to encoder().outputs.single())) {
            val items=b.vector(1); b.ref(signature+4+field*4,items)
            val item=b.table(2,setOf(0,1)); b.ref(items+4,item)
            b.ref(item+4,b.string(tensor.signatureName)); b.int(item+8,tensor.index)
        }
        b.ref(signature+12,b.string("serving_default"))
        return b.bytes.copyOf(b.position)
    }
    private class FixtureBuffer {
        val bytes=ByteArray(4096); var position=8
        private fun align(n: Int) { while (position % n != 0) position++ }
        fun int(at: Int, value: Int) { repeat(4) { bytes[at+it]=(value ushr (it*8)).toByte() } }
        private fun short(at: Int, value: Int) { bytes[at]=value.toByte(); bytes[at+1]=(value ushr 8).toByte() }
        fun ref(at: Int, target: Int) { int(at,target-at) }
        fun table(fields: Int, present: Set<Int>): Int {
            align(2); val vtable=position; position+=4+fields*2
            align(4); val table=position; position+=4+fields*4
            short(vtable,4+fields*2); short(vtable+2,4+fields*4)
            repeat(fields) { short(vtable+4+it*2,if (it in present) 4+it*4 else 0) }
            int(table,table-vtable); return table
        }
        fun vector(count: Int): Int { align(4); val at=position; position+=4+count*4; int(at,count); return at }
        fun string(value: String): Int { align(4); val at=position; position+=5+value.length; int(at,value.length)
            value.encodeToByteArray().copyInto(bytes,at+4); return at }
    }
}
