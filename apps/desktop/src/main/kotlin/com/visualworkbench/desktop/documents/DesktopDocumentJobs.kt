package com.visualworkbench.desktop.documents

import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*
import kotlinx.coroutines.CancellationException

internal class DesktopDocumentJobs(
    runtime: DesktopDocumentRuntime, pcRedaction: Boolean, retained: AutoCloseable, pdfium: Boolean = true,
) : DocumentJobs by JoinedDocumentJobs(NativeDocumentBackend(runtime, pcRedaction, pdfium, retained))

private class NativeDocumentBackend(
    private val runtime: DesktopDocumentRuntime,
    override val pcRedaction: Boolean,
    private val pdfium: Boolean,
    private val retained: AutoCloseable,
) : DocumentBackend {
    private val runtimeRetirement = DocumentNativeRetirement(
        prove = { runtime.retryRetirement() }, destroy = { runtime.destroy() }, initiallyRequired = true,
    )
    override fun call(): DocumentCall = object : DocumentCall {
        private val cancellation = DocumentJobCancellation()
        private val retirement = DocumentNativeRetirement(
            prove = { runtime.retryRetirement() }, destroy = { cancellation.destroy() },
        )
        private var heldSource: ByteArray? = null
        override fun cancel(): Unit = retirement.cancel { cancellation.cancel() }
        override suspend fun execute(request: DocumentRequest, source: ByteArray): DocumentReply {
            heldSource = source
            if (source.isEmpty()) throw DocumentFailure(DocumentFailureKind.Invalid)
            if (source.size > if (documentSvgOperation(request.operation)) 4 * 1024 * 1024 else 64 * 1024 * 1024)
                throw DocumentFailure(DocumentFailureKind.Limit)
            if (!documentSvgOperation(request.operation) && !pdfium) throw DocumentFailure(DocumentFailureKind.Unavailable)
            val operation = request.operation
            if (operation is DocumentOperation.Copy && operation.kind == DocumentCopy.PcRedaction && !pcRedaction)
                throw DocumentFailure(DocumentFailureKind.Unsupported)
            try {
                val text = documentRequestJson(request)
                val expected = documentSourceHash(source)
                retirement.beforeRun()
                val result = runtime.run(text, source, cancellation)
                return documentReply(validateDocumentJobResult(text, expected, result))
            } catch (error: DocumentJobException) {
                throw nativeDocumentFailure(error)
            } catch (error: DocumentFailure) {
                throw error
            } catch (error: CancellationException) {
                throw error
            } catch (_: Exception) {
                throw DocumentFailure(DocumentFailureKind.Worker)
            }
        }
        override suspend fun retire() {
            retirement.retire()
            heldSource = null
        }
    }
    override suspend fun close() {
        runtimeRetirement.retire()
        try { retained.close() } catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Isolation) }
    }
}

internal fun documentSvgOperation(operation: DocumentOperation): Boolean = when (operation) {
    DocumentOperation.SvgInfo -> true
    is DocumentOperation.Tile -> operation.kind == EditableKind.Svg
    is DocumentOperation.Copy -> operation.kind == DocumentCopy.SvgAnnotations
    else -> false
}

internal fun nativeDocumentFailure(error: DocumentJobException): DocumentFailure = DocumentFailure(when (error) {
    is DocumentJobException.Unavailable -> DocumentFailureKind.Unavailable
    is DocumentJobException.Invalid -> DocumentFailureKind.Invalid
    is DocumentJobException.Limit -> DocumentFailureKind.Limit
    is DocumentJobException.Busy -> DocumentFailureKind.Busy
    is DocumentJobException.Cancelled -> DocumentFailureKind.Cancelled
    is DocumentJobException.Timeout -> DocumentFailureKind.Timeout
    is DocumentJobException.Isolation -> DocumentFailureKind.Isolation
    is DocumentJobException.Verification -> DocumentFailureKind.Verification
    is DocumentJobException.Password -> DocumentFailureKind.Password
    is DocumentJobException.Unsupported -> DocumentFailureKind.Unsupported
    is DocumentJobException.Malformed -> DocumentFailureKind.Malformed
    is DocumentJobException.Worker -> DocumentFailureKind.Worker
})
