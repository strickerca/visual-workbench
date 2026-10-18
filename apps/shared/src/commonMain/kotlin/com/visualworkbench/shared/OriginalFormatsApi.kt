package com.visualworkbench.shared

public data class OriginalFormatInfo(
    public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val format: String, public val sourceBitDepth: UByte,
    public val untagged: Boolean, public val firstTiffPageOnly: Boolean,
    public val flattenedComposite: Boolean = false, public val rawPreview: Boolean = false,
)
public interface WorkbenchOriginalFormats {
    public suspend fun originalFormatInfo(binding: WorkflowBinding): OriginalFormatInfo
}
/** Explicit embedded-preview intake. Staged source bytes still require native bounded RAW admission. */
public expect suspend fun createRawPreviewProject(options: CreateFileProject): WorkbenchProject

public fun validateOriginalFormatInfo(value: OriginalFormatInfo, binding: WorkflowBinding): Unit {
    val composite = value.format in setOf("psd", "psb")
    val preview = value.format in setOf("dng", "cr2", "raw-preview")
    if (value.binding != binding || value.sourceAssetId.length != 64 || value.sourceAssetId.any { it !in "0123456789abcdef" } ||
        value.format !in setOf("png", "jpeg", "webp", "heic", "heif", "avif", "bmp", "tiff", "psd", "psb", "dng", "cr2", "raw-preview") ||
        value.sourceBitDepth.toInt() !in listOf(8, 16) || (value.firstTiffPageOnly && value.format != "tiff") ||
        value.flattenedComposite != composite || value.rawPreview != preview || (preview && value.sourceBitDepth.toInt() != 8))
        throw IllegalArgumentException("Original format receipt does not match its owned request")
}
public fun originalFormatNotice(value: OriginalFormatInfo): String? = when {
    value.firstTiffPageOnly -> "Only the first TIFF page is displayed and exported. All original pages remain in the unchanged source file."
    value.flattenedComposite -> "Flattened embedded composite only. Original layers remain in the unchanged source; layered round trips are unsupported."
    value.rawPreview -> "Rendered embedded RAW preview only; sensor data is retained unchanged. RAW development and full-resolution export are unavailable."
    else -> null
}

/** Full revision check after the worker returns, before a platform adopts a notice. */
public fun originalFormatRevisionMatches(info: ProjectInfo, binding: WorkflowBinding): Boolean =
    info.projectId == binding.projectId && info.hostSeq == binding.hostSeq && info.stateHash == binding.stateHash &&
        binding.documentId in info.documentIds
