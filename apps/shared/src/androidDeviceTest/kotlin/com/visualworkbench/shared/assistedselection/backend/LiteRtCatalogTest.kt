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
            refuses { inspectFlatbuffer(spec.copy(operators=listOf(18 to 1))) }
            file.writeBytes(valid.copyOf(12)); refuses { inspectFlatbuffer(spec.copy(bytes=12)) }
            val corrupt=valid.copyOf(); repeat(4) { corrupt[it]=127 }; file.writeBytes(corrupt)
            refuses { inspectFlatbuffer(spec) }
        } finally { file.delete() }
    }

    @Test fun everyInternalTensorMustBeStaticNonvariableNonresourceAndUseAnExistingBuffer() {
        checkFixture(FixturePolicy(),accepted=true)
        listOf(FixturePolicy(internalSignature=listOf(-1)),FixturePolicy(internalSignature=listOf(2)),
            FixturePolicy(internalShape=listOf(-1)),FixturePolicy(internalShape=listOf(16777217)),
            FixturePolicy(internalVariable=true),FixturePolicy(internalType=13),FixturePolicy(internalType=14),
            FixturePolicy(internalBuffer=1)).forEach { checkFixture(it) }
    }

    @Test fun buffersAndOperatorStorageIndicesOptionsAndMutabilityFollowTheSameStaticPolicy() {
        checkFixture(FixturePolicy(operatorInput=-1),accepted=true)
        listOf(FixturePolicy(bufferOffset=1),FixturePolicy(bufferSize=1),FixturePolicy(overflowData=true),
            FixturePolicy(noBuffers=true),FixturePolicy(noOperators=true),FixturePolicy(operatorInput=-2),
            FixturePolicy(operatorInput=3),FixturePolicy(operatorOutput=-1),FixturePolicy(operatorOutput=3),
            FixturePolicy(intermediate=-1),FixturePolicy(intermediate=3),FixturePolicy(customOptions=true),
            FixturePolicy(mutableInput=true),FixturePolicy(externalOptionsOffset=1),FixturePolicy(externalOptionsSize=1))
            .forEach { checkFixture(it) }
    }

    @Test fun exactlyOneNamedSignatureAndUniqueGraphIoAreRequired() {
        checkFixture(FixturePolicy(extraSignature=true))
        checkFixture(FixturePolicy(duplicateInputs=true))
        checkFixture(FixturePolicy(internalSignature=listOf(1)),accepted=true)
    }

    private fun checkFixture(policy: FixturePolicy, accepted: Boolean = false) {
        val file=Files.createTempFile("vw-tflite-static-test",".tflite").toFile()
        try {
            val bytes=metadataFixture(policy); file.writeBytes(bytes)
            val spec=encoder().copy(file=file,bytes=bytes.size.toLong())
            if (accepted) inspectFlatbuffer(spec) else refuses { inspectFlatbuffer(spec) }
        } finally { file.delete() }
    }

    private fun refuses(block: () -> Unit) {
        try { block(); fail("Untrusted metadata admitted") } catch (_: SelectionFailure) { }
    }
    private fun tensor(name: String, index: Int, type: String, vararg shape: Int): TensorSpec =
        TensorSpec(name,index,name,name,type,shape.toList())
    private fun encoder(): ModelSpec = ModelSpec(File("unused"),1,"","serving_default",
        listOf(tensor("image",0,"float32",1,3,1024,1024)),listOf(tensor("image_embeddings",1,"float32",1,256,64,64)),listOf(0 to 1))
    private fun decoder(): ModelSpec = ModelSpec(File("unused"),1,"","serving_default",listOf(
        tensor("image_embeddings",0,"float32",1,256,64,64),tensor("point_coords",1,"float32",1,66,2),
        tensor("point_labels",2,"int32",1,66),tensor("point_valid",3,"int32",1,66),tensor("token_count",4,"int32",1)),
        listOf(tensor("mask_logits",5,"float32",1,1,256,256),tensor("score",6,"float32",1,1)),emptyList())

    /** Structural metadata fixture only, no weights/executable inference graph.
     * Tiny forward-offset builder avoids importing a model/runtime test engine. */
    private data class FixturePolicy(
        val internalShape: List<Int> = listOf(1), val internalSignature: List<Int> = emptyList(),
        val internalVariable: Boolean = false, val internalType: Int = 0, val internalBuffer: Int = 0,
        val bufferOffset: Long = 0, val bufferSize: Long = 0, val overflowData: Boolean = false,
        val operatorInput: Int = 0, val operatorOutput: Int = 1, val intermediate: Int? = null,
        val customOptions: Boolean = false, val mutableInput: Boolean = false,
        val externalOptionsOffset: Long = 0, val externalOptionsSize: Long = 0,
        val extraSignature: Boolean = false, val duplicateInputs: Boolean = false,
        val noBuffers: Boolean = false, val noOperators: Boolean = false,
    )
    private fun metadataFixture(policy: FixturePolicy = FixturePolicy()): ByteArray {
        val b=FixtureBuffer(); val model=b.table(8,setOf(0,1,2,4,7)); b.int(0,model); b.int(4,0x334c4654)
        b.int(model+4,3)
        val codes=b.vector(1); b.ref(model+8,codes)
        val code=b.table(4,setOf(2,3)); b.ref(codes+4,code); b.int(code+12,1); b.int(code+16,0)
        val graphs=b.vector(1); b.ref(model+12,graphs)
        val graph=b.table(4,setOf(0,1,2,3)); b.ref(graphs+4,graph)
        val tensors=b.vector(3); b.ref(graph+4,tensors)
        val internal=tensor("internal",2,"float32",*policy.internalShape.toIntArray())
        (encoder().inputs+encoder().outputs+internal).forEachIndexed { index,tensor ->
            val table=b.table(8,setOf(0,1,2,3,5,7)); b.ref(tensors+4+index*4,table)
            val shape=b.vector(tensor.shape.size); b.ref(table+4,shape)
            tensor.shape.forEachIndexed { i,n -> b.int(shape+4+i*4,n) }
            b.int(table+8,if (index == 2) policy.internalType else 0)
            b.int(table+12,if (index == 2) policy.internalBuffer else 0)
            b.ref(table+16,b.string(tensor.tensorName))
            b.int(table+24,if (index == 2 && policy.internalVariable) 1 else 0)
            val signature=if (index == 2) policy.internalSignature else tensor.shape
            val dimensions=b.vector(signature.size); b.ref(table+32,dimensions)
            signature.forEachIndexed { i,n -> b.int(dimensions+4+i*4,n) }
        }
        val inputs=b.vector(if (policy.duplicateInputs) 2 else 1); b.int(inputs+4,0)
        if (policy.duplicateInputs) b.int(inputs+8,0)
        b.ref(graph+8,inputs)
        val outputs=b.vector(1); b.int(outputs+4,1); b.ref(graph+12,outputs)
        val operators=b.vector(if (policy.noOperators) 0 else 1); b.ref(graph+16,operators)
        if (!policy.noOperators) {
            val present=mutableSetOf(0,1,2,8)
            if (policy.customOptions) present+=5
            if (policy.mutableInput) present+=7
            if (policy.externalOptionsOffset != 0L) present+=9
            if (policy.externalOptionsSize != 0L) present+=10
            val operator=b.table(12,present); b.ref(operators+4,operator)
            val ins=b.vector(1); b.ref(operator+8,ins); b.int(ins+4,policy.operatorInput)
            val outs=b.vector(1); b.ref(operator+12,outs); b.int(outs+4,policy.operatorOutput)
            val intermediates=b.vector(if (policy.intermediate == null) 0 else 1); b.ref(operator+36,intermediates)
            policy.intermediate?.let { b.int(intermediates+4,it) }
            if (policy.customOptions) b.ref(operator+24,b.vector(1,1))
            if (policy.mutableInput) { val flags=b.vector(1,1); b.ref(operator+32,flags); b.bytes[flags+4]=1 }
            if (policy.externalOptionsOffset != 0L) b.long(operator+40,policy.externalOptionsOffset)
            if (policy.externalOptionsSize != 0L) b.long(operator+44,policy.externalOptionsSize)
        }
        val buffers=b.vector(if (policy.noBuffers) 0 else 1); b.ref(model+20,buffers)
        if (!policy.noBuffers) {
            val present=mutableSetOf(0)
            if (policy.bufferOffset != 0L) present+=1
            if (policy.bufferSize != 0L) present+=2
            val buffer=b.table(4,present); b.ref(buffers+4,buffer)
            val data=b.vector(0,1); b.ref(buffer+4,data)
            if (policy.overflowData) b.int(data,Int.MAX_VALUE)
            if (policy.bufferOffset != 0L) b.long(buffer+8,policy.bufferOffset)
            if (policy.bufferSize != 0L) b.long(buffer+12,policy.bufferSize)
        }
        val signatures=b.vector(if (policy.extraSignature) 2 else 1); b.ref(model+32,signatures)
        repeat(if (policy.extraSignature) 2 else 1) { index ->
            val signature=b.table(5,setOf(0,1,2)); b.ref(signatures+4+index*4,signature)
            for ((field,tensor) in listOf(0 to encoder().inputs.single(),1 to encoder().outputs.single())) {
                val items=b.vector(1); b.ref(signature+4+field*4,items)
                val item=b.table(2,setOf(0,1)); b.ref(items+4,item)
                b.ref(item+4,b.string(tensor.signatureName)); b.int(item+8,tensor.index)
            }
            b.ref(signature+12,b.string(if (index == 0) "serving_default" else "other"))
        }
        return b.bytes.copyOf(b.position)
    }
    private class FixtureBuffer {
        val bytes=ByteArray(4096); var position=8
        private fun align(n: Int) { while (position % n != 0) position++ }
        fun int(at: Int, value: Int) { repeat(4) { bytes[at+it]=(value ushr (it*8)).toByte() } }
        fun long(at: Int, value: Long) { repeat(8) { bytes[at+it]=(value ushr (it*8)).toByte() } }
        private fun short(at: Int, value: Int) { bytes[at]=value.toByte(); bytes[at+1]=(value ushr 8).toByte() }
        fun ref(at: Int, target: Int) { int(at,target-at) }
        fun table(fields: Int, present: Set<Int>): Int {
            align(2); val vtable=position; position+=4+fields*2
            align(4); val table=position; position+=4+fields*4
            short(vtable,4+fields*2); short(vtable+2,4+fields*4)
            repeat(fields) { short(vtable+4+it*2,if (it in present) 4+it*4 else 0) }
            int(table,table-vtable); return table
        }
        fun vector(count: Int, elementSize: Int = 4): Int { align(4); val at=position; position+=4+count*elementSize; int(at,count); return at }
        fun string(value: String): Int { align(4); val at=position; position+=5+value.length; int(at,value.length)
            value.encodeToByteArray().copyInto(bytes,at+4); return at }
    }
}
