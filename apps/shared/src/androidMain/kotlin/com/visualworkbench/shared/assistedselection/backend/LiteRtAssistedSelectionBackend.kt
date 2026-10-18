package com.visualworkbench.shared.assistedselection.backend

import android.content.Context
import android.os.Build
import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import java.io.FileDescriptor
import java.io.File
import java.util.UUID
import java.security.MessageDigest
import com.google.ai.edge.litert.Accelerator
import com.google.ai.edge.litert.CompiledModel
import com.google.ai.edge.litert.Environment
import com.google.ai.edge.litert.TensorBuffer
import com.google.ai.edge.litert.TensorType
import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlin.coroutines.CoroutineContext

/** Standalone LiteRT 2.3.0 only. Root supplies a closed reviewed catalog, actual
 * borrowed project and current visible source. No model entry is enabled here.
 * The read-only, thread-safe callback must return null whenever the ACTUAL
 * project reference differs from the captured project, during disposal/reopen,
 * or after a page/epoch/binding change. It must not reenter close or mutation. */
public fun liteRtAssistedSelectionBackend(context: Context, catalog: LiteRtReviewedCatalog,
    project: WorkbenchProject, currentSource: () -> AssistedSelectionSource?, assumeUntaggedSrgb: Boolean = false): AssistedSelectionBackend =
    LiteRtBackend(context.applicationContext,catalog,currentSource,assumeUntaggedSrgb) { assistedSourcePixels(project) }

/** The internal reader injection is for contract tests only; production always
 * acquires the root's actual project-backed source capability above. */
internal class LiteRtBackend(private val context: Context, private val catalog: LiteRtReviewedCatalog,
    private val currentSource: () -> AssistedSelectionSource?, private val assume: Boolean,
    private val reader: () -> WorkbenchAssistedSourcePixels) : AssistedSelectionBackend {
    override suspend fun admit(pins: AssistedSelectionPins): AssistedSelectionRuntime {
        val task=currentCoroutineContext()
        if ("arm64-v8a" !in Build.SUPPORTED_ABIS) unsupported()
        // Metadata/hash/APK capability checks only. Do not initialize LiteRT
        // classes, an Environment, a worker, weights or source pixels here.
        val verified=withContext(Dispatchers.IO) { verifyCatalog(context,catalog,pins,task) }
        task.ensureActive()
        return LiteRtRuntime(context,catalog,verified,currentSource,assume,reader)
    }
}

internal class AffineWorker {
    private var thread: Thread? = null
    private val executor=Executors.newSingleThreadExecutor { task -> Thread(task,"vw-assisted-litert").also { thread=it } }
    val dispatcher: ExecutorCoroutineDispatcher = executor.asCoroutineDispatcher()
    suspend fun <T> run(task: CoroutineContext, block: suspend () -> T): T = withContext(NonCancellable) {
        withContext(dispatcher) { block().also { task.ensureActive() } }
    }
    suspend fun joinClose() {
        executor.shutdown(); dispatcher.close()
        withContext(Dispatchers.IO) {
            while (!executor.awaitTermination(1,TimeUnit.SECONDS)) { /* actual joined retirement */ }
            thread?.join()
        }
    }
}

internal class EngineOwner {
    val handles: MutableList<AutoCloseable> = mutableListOf()
    var lease: AssistedSourcePixelsLease? = null
    var image: FloatArray? = null
    var embedding: FloatArray? = null
    var encoder: CompiledModel? = null
    var decoder: CompiledModel? = null
    var encoderInputs: Map<String,TensorBuffer> = emptyMap()
    var encoderOutputs: Map<String,TensorBuffer> = emptyMap()
    var decoderInputs: Map<String,TensorBuffer> = emptyMap()
    var decoderOutputs: Map<String,TensorBuffer> = emptyMap()
    fun <T: AutoCloseable> own(value: T): T { handles.add(value); return value }
    suspend fun close() {
        image=null; embedding=null
        lease?.let { it.close(); lease=null }
        // Remove only after successful retirement. If close fails, every
        // dependent earlier handle and the exact failed handle stay reachable.
        closeTo(0)
        encoder=null; decoder=null
        encoderInputs=emptyMap(); encoderOutputs=emptyMap(); decoderInputs=emptyMap(); decoderOutputs=emptyMap()
    }
    fun closeTo(mark: Int) {
        if (mark !in 0..handles.size) corrupt()
        while (handles.size > mark) { handles.last().close(); handles.removeAt(handles.lastIndex) }
    }
}

private class LiteRtRuntime(private val context: Context, private val catalog: LiteRtReviewedCatalog,
    private val verified: VerifiedLiteRt, private val currentSource: () -> AssistedSelectionSource?, private val assume: Boolean,
    private val readerFactory: () -> WorkbenchAssistedSourcePixels) : AssistedSelectionRuntime {
    override val admittedPins: AssistedSelectionPins = verified.entry.pins
    override val executions: Set<AssistedSelectionExecution> = if (verified.gpuEligible)
        setOf(AssistedSelectionExecution.Cpu,AssistedSelectionExecution.Gpu) else setOf(AssistedSelectionExecution.Cpu)
    override val workingSetBytes: ULong = verified.workingSet
    private val gate=Mutex()
    private var worker: AffineWorker? = null
    private var reader: WorkbenchAssistedSourcePixels? = null
    private var owner: EngineOwner? = null
    private var session: LiteRtSession? = null
    private var closing=false
    private var closed=false
    private var encoded=false

    private fun checkCurrent(source: AssistedSelectionSource) {
        if (currentSource() != source) throw SelectionFailure(SelectionFailureKind.Conflict)
    }
    private fun requireOpen() { if (closing || closed) throw SelectionFailure(SelectionFailureKind.Closed) }
    private fun sourceReader(): WorkbenchAssistedSourcePixels = reader ?: readerFactory().also { reader=it }
    private suspend fun <T> affine(task: CoroutineContext, block: suspend () -> T): T = withContext(NonCancellable) {
        val value=worker ?: AffineWorker().also { worker=it }
        // NonCancellable joins a non-cooperative native run. Explicit captured
        // caller checks surround runs/row loops; cancellation never abandons it.
        value.run(task,block)
    }

    override suspend fun encode(source: AssistedSelectionSource, execution: AssistedSelectionExecution): AssistedSelectionSession {
        val task=currentCoroutineContext()
        return gate.withLock {
            requireOpen()
            if (execution !in executions) throw SelectionFailure(SelectionFailureKind.Invalid)
            if (encoded || owner != null) throw SelectionFailure(SelectionFailureKind.Backpressure)
            try { source.requireAssistedOriginal(POOL_BYTES); samSize(source.document.width,source.document.height) }
            catch (failure: SelectionFailure) {
                if (failure.kind == SelectionFailureKind.Unsupported) throw SelectionFailure(SelectionFailureKind.Invalid,cause=failure)
                throw failure
            }
            if (source.document.width.toULong()*source.document.height.toULong() > verified.entry.memory.maximumSourcePixels.toULong())
                throw SelectionFailure(SelectionFailureKind.Memory,workingSetBytes,POOL_BYTES)
            checkCurrent(source); task.ensureActive()
            var unavailableAccelerator=false
            var interimRetirementFailed=false
            try { affine(task) {
                val partial=EngineOwner(); owner=partial
                val actual=verifyCatalog(context,catalog,admittedPins,task)
                if (actual.workingSet != workingSetBytes) corrupt()
                checkCurrent(source); task.ensureActive()
                val pixels=sourceReader().acquire(source,assume,POOL_BYTES)
                partial.lease=pixels
                if (pixels.source != source) throw SelectionFailure(SelectionFailureKind.Conflict)
                checkCurrent(source); task.ensureActive()
                partial.image=samImage(pixels.rgba,source.document.width,source.document.height,task)
                pixels.close(); partial.lease=null
                checkCurrent(source); task.ensureActive()
                val environment=partial.own(Environment.create(context,emptyMap(),enableCompilerCache=false))
                val accelerator=if (execution == AssistedSelectionExecution.Gpu) Accelerator.GPU else Accelerator.CPU
                // Eligibility at admission was OS+packaged-library evidence.
                // Actual provider discovery occurs only AFTER reservation.
                if (accelerator !in environment.getAvailableAccelerators()) { unavailableAccelerator=true; unsupported() }
                // LiteRT2.3.0 exposes no public typed exception status getter.
                // Unknown create/compile failures cannot safely become fallback:
                // only observed provider absence above is Unsupported initialization.
                val options=CompiledModel.Options(accelerator)
                val environmentMark=partial.handles.size
                partial.encoder=openModel(actual.encoder,options,environment,partial,task)
                partial.encoderInputs=buffers(partial.encoder!!,actual.encoder,true,partial)
                partial.encoderOutputs=buffers(partial.encoder!!,actual.encoder,false,partial)
                partial.encoderInputs.getValue("image").writeFloat(partial.image!!)
                partial.image=null; task.ensureActive(); checkCurrent(source)
                partial.encoder!!.run(signatureMap(actual.encoder.inputs,partial.encoderInputs),
                    signatureMap(actual.encoder.outputs,partial.encoderOutputs),actual.encoder.signature)
                task.ensureActive(); checkCurrent(source)
                val embedding=partial.encoderOutputs.getValue("image_embeddings").readFloat()
                if (embedding.size != EMBEDDING_ELEMENTS || embedding.any { !it.isFinite() }) corrupt()
                partial.embedding=embedding
                // Retain the actual embedding but retire encoder-only buffers,
                // model mappings and snapshot descriptors before loading decoder.
                try { partial.closeTo(environmentMark) }
                catch (failure: Throwable) { interimRetirementFailed=true; closing=true; throw failure }
                partial.encoder=null; partial.encoderInputs=emptyMap(); partial.encoderOutputs=emptyMap()
                task.ensureActive(); checkCurrent(source)
                partial.decoder=openModel(actual.decoder,options,environment,partial,task)
                partial.decoderInputs=buffers(partial.decoder!!,actual.decoder,true,partial)
                partial.decoderOutputs=buffers(partial.decoder!!,actual.decoder,false,partial)
                partial.decoderInputs.getValue("image_embeddings").writeFloat(embedding)
                partial.embedding=null
                task.ensureActive(); checkCurrent(source)
                LiteRtSession(source,execution,this,partial,actual.decoder).also { session=it; encoded=true }
            } } catch (failure: Throwable) {
                if (interimRetirementFailed) throw SelectionFailure(SelectionFailureKind.Storage,cause=failure)
                // Also catches cancellation at the final worker handoff: a
                // produced-but-undelivered session remains owned by this runtime.
                try { withContext(NonCancellable) {
                    withContext(worker!!.dispatcher) {
                        owner?.close(); reader?.retireUndelivered()
                        session?.retired=true; session=null; owner=null
                    }
                } } catch (retirement: Throwable) {
                    closing=true; throw SelectionFailure(SelectionFailureKind.Storage,cause=retirement)
                }
                if (failure is SelectionFailure && failure.kind == SelectionFailureKind.Unsupported && !unavailableAccelerator)
                    throw SelectionFailure(SelectionFailureKind.Corrupt,cause=failure)
                throw failure
            }
        }
    }

    private fun openModel(spec: ModelSpec, options: CompiledModel.Options, environment: Environment,
        partial: EngineOwner, task: CoroutineContext): CompiledModel {
        val raw=partial.own(OsDescriptor(Os.open(spec.file.absolutePath,
            OsConstants.O_RDONLY or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW or OsConstants.O_NONBLOCK,0)))
        val attributes=Os.fstat(raw.descriptor)
        if (!OsConstants.S_ISREG(attributes.st_mode) || attributes.st_size != spec.bytes) corrupt()
        val snapshot=partial.own(ModelSnapshot(File(context.cacheDir,"vw-assisted-${UUID.randomUUID()}.tmp")))
        snapshot.create(); snapshot.unlink()
        val sha=MessageDigest.getInstance("SHA-256"); val scratch=ByteArray(65536); var count=0L
        while (true) {
            task.ensureActive(); val length=Os.read(raw.descriptor,scratch,0,scratch.size)
            if (length == 0) break
            count+=length; if (count > spec.bytes) corrupt(); sha.update(scratch,0,length)
            var written=0
            while (written < length) {
                val bytes=Os.write(snapshot.writer!!,scratch,written,length-written)
                if (bytes <= 0) throw SelectionFailure(SelectionFailureKind.Storage)
                written+=bytes
            }
        }
        val actual=sha.digest().joinToString("") { "%02x".format(it) }
        if (count != spec.bytes || actual != spec.sha256) corrupt()
        val writerDuplicate=partial.own(ParcelFileDescriptor.dup(snapshot.writer!!))
        val readOnly=partial.own(OsDescriptor(Os.open("/proc/self/fd/${writerDuplicate.fd}",
            OsConstants.O_RDONLY or OsConstants.O_CLOEXEC,0)))
        val descriptor=partial.own(ParcelFileDescriptor.dup(readOnly.descriptor))
        writerDuplicate.close(); snapshot.closeWriter()
        // Native code sees only the verified, unlinked private snapshot after
        // ALL writable descriptors close. Later source inode/path mutation cannot
        // alter these model bytes. Read-only descriptors outlive the model.
        val model=partial.own(CompiledModel.create("/proc/self/fd/${descriptor.fd}",options,environment))
        (spec.inputs.map { true to it }+spec.outputs.map { false to it }).forEach { (input,tensor) ->
            val type=if (input) model.getInputTensorType(tensor.signatureName,spec.signature)
                else model.getOutputTensorType(tensor.signatureName,spec.signature)
            val expected=if (tensor.dtype == "float32") TensorType.ElementType.FLOAT else TensorType.ElementType.INT
            val layout=type.layout ?: corrupt()
            if (type.elementType != expected || layout.dimensions != tensor.shape || layout.hasStrides) corrupt()
            val requirements=if (input) model.getInputBufferRequirements(tensor.signatureName,spec.signature)
                else model.getOutputBufferRequirements(tensor.signatureName,spec.signature)
            if (requirements.bufferSize < tensor.bytes || requirements.bufferSize.toLong() > tensor.bytes.toLong()*2+65536 ||
                requirements.strides.isNotEmpty()) corrupt()
        }
        return model
    }

    private fun buffers(model: CompiledModel, spec: ModelSpec, input: Boolean, partial: EngineOwner): Map<String,TensorBuffer> =
        (if (input) spec.inputs else spec.outputs).associate { tensor ->
            tensor.role to partial.own(if (input) model.createInputBuffer(tensor.signatureName,spec.signature)
                else model.createOutputBuffer(tensor.signatureName,spec.signature))
        }

    suspend fun propose(value: LiteRtSession, partial: EngineOwner, decoder: ModelSpec,
        request: AssistedSelectionPromptRequest): AssistedSelectionMask {
        val task=currentCoroutineContext()
        return gate.withLock {
            requireOpen()
            if (session !== value || owner !== partial || value.retired || request.source != value.source)
                throw SelectionFailure(SelectionFailureKind.Conflict)
            checkCurrent(value.source); task.ensureActive()
            affine(task) {
                val prompt=samPrompt(request)
                partial.decoderInputs.getValue("point_coords").writeFloat(prompt.coordinates)
                partial.decoderInputs.getValue("point_labels").writeInt(prompt.labels)
                partial.decoderInputs.getValue("point_valid").writeInt(prompt.valid)
                partial.decoderInputs.getValue("token_count").writeInt(prompt.count)
                task.ensureActive(); checkCurrent(value.source)
                partial.decoder!!.run(signatureMap(decoder.inputs,partial.decoderInputs),
                    signatureMap(decoder.outputs,partial.decoderOutputs),decoder.signature)
                task.ensureActive(); checkCurrent(value.source)
                val logits=partial.decoderOutputs.getValue("mask_logits").readFloat()
                val score=partial.decoderOutputs.getValue("score").readFloat()
                val coverage=samCoverage(logits,score,value.source.document.width,value.source.document.height,task)
                task.ensureActive(); checkCurrent(value.source)
                AssistedSelectionMask(value.source,request.sequence,value.source.document.width,value.source.document.height,coverage)
            }
        }
    }

    suspend fun closeSession(value: LiteRtSession, partial: EngineOwner) = withContext(NonCancellable) {
        gate.withLock {
            if (value.retired) return@withLock
            val affine=worker ?: corrupt()
            try { withContext(affine.dispatcher) { partial.close(); reader?.retireUndelivered() } }
            catch (failure: Throwable) { closing=true; throw failure }
            value.retired=true
            if (owner === partial) owner=null
            if (session === value) session=null
        }
    }

    override suspend fun close(): Unit = withContext(NonCancellable) {
        closing=true
        gate.withLock {
            if (closed) return@withLock
            worker?.let { affine ->
                withContext(affine.dispatcher) { owner?.close(); reader?.retireUndelivered() }
                owner=null; session?.retired=true; session=null
                affine.joinClose(); worker=null
            } ?: reader?.retireUndelivered()
            closed=true
        }
    }
}

private class LiteRtSession(override val source: AssistedSelectionSource, override val execution: AssistedSelectionExecution,
    private val runtime: LiteRtRuntime, private val owner: EngineOwner, private val decoder: ModelSpec) : AssistedSelectionSession {
    var retired=false
    override suspend fun propose(request: AssistedSelectionPromptRequest): AssistedSelectionMask = runtime.propose(this,owner,decoder,request)
    override suspend fun close(): Unit = runtime.closeSession(this,owner)
}
private fun signatureMap(specs: List<TensorSpec>, buffers: Map<String,TensorBuffer>): Map<String,TensorBuffer> =
    specs.associate { it.signatureName to buffers.getValue(it.role) }

private class OsDescriptor(val descriptor: FileDescriptor) : AutoCloseable {
    private var closed=false
    override fun close() { if (!closed) { Os.close(descriptor); closed=true } }
}

/** No cache entry survives. Creation is exclusive, unlink precedes all bulk
 * writes, and failed close/unlink retains the exact descriptor/path here. */
private class ModelSnapshot(private val path: File) : AutoCloseable {
    var writer: FileDescriptor? = null
        private set
    private var linked=false
    fun create() {
        writer=Os.open(path.absolutePath,OsConstants.O_CREAT or OsConstants.O_EXCL or OsConstants.O_RDWR or
            OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW,384)
        linked=true
    }
    fun unlink() { if (linked) { Os.remove(path.absolutePath); linked=false } }
    fun closeWriter() { writer?.let { Os.close(it); writer=null } }
    override fun close() { closeWriter(); unlink() }
}
