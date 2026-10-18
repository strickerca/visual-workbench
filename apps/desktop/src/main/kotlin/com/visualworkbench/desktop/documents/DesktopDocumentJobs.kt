package com.visualworkbench.desktop.documents

import com.visualworkbench.bindings.core.*
import com.visualworkbench.shared.*

internal class DesktopDocumentJobs(
    runtime: DesktopDocumentRuntime, pcRedaction: Boolean, retained: AutoCloseable, pdfium: Boolean = true,
) : DocumentJobs by JoinedDocumentJobs(NativeDocumentBackend(runtime, pcRedaction, pdfium, retained))

private class NativeDocumentBackend(
    private val runtime: DesktopDocumentRuntime,
    override val pcRedaction: Boolean,
    private val pdfium: Boolean,
    private val retained: AutoCloseable,
) : DocumentBackend {
    private var destroyed = false
    override fun call(): DocumentCall = object : DocumentCall {
        private val cancellation = DocumentJobCancellation()
        private val state = Any()
        private var uncertain = false
        private var retired = false
        private var heldSource: ByteArray? = null
        override fun cancel() = synchronized(state) {
            try { if (!retired) cancellation.cancel() }
            catch (_: Exception) { uncertain = true }
        }
        override suspend fun execute(request: DocumentRequest, source: ByteArray): DocumentReply {
            heldSource = source
            if (!documentSvgOperation(request.operation) && !pdfium) throw DocumentFailure(DocumentFailureKind.Unavailable)
            if (request.operation is DocumentOperation.Copy && request.operation.kind == DocumentCopy.PcRedaction && !pcRedaction)
                throw DocumentFailure(DocumentFailureKind.Unsupported)
            try {
                val text = documentRequestJson(request)
                val expected = documentSourceHash(source)
                val result = runtime.run(text, source, cancellation)
                return documentReply(validateDocumentJobResult(text, expected, result))
            } catch (error: DocumentJobException) {
                val failure = nativeDocumentFailure(error)
                if (failure.kind == DocumentFailureKind.Isolation) synchronized(state) { uncertain = true }
                throw failure
            }
        }
        override suspend fun retire() {
            // The root ABI must add a joined-retirement retry to recover from
            // Isolation. Until then preserve this exact runtime/token/pin owner;
            // destroy is never used as a substitute for retirement evidence.
            synchronized(state) {
                if (uncertain) throw DocumentFailure(DocumentFailureKind.Isolation)
                if (!retired) { cancellation.destroy(); retired = true; heldSource = null }
            }
        }
    }
    override suspend fun close() {
        if (!destroyed) {
            try { runtime.destroy(); destroyed = true }
            catch (_: Exception) { throw DocumentFailure(DocumentFailureKind.Isolation) }
        }
        retained.close()
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
