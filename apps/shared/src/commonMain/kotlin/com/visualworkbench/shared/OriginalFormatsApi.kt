package com.visualworkbench.shared

public data class OriginalFormatInfo(
    public val binding: WorkflowBinding, public val sourceAssetId: String,
    public val format: String, public val sourceBitDepth: UByte,
    public val untagged: Boolean, public val firstTiffPageOnly: Boolean,
)
public interface WorkbenchOriginalFormats {
    public suspend fun originalFormatInfo(binding: WorkflowBinding): OriginalFormatInfo
}
public fun validateOriginalFormatInfo(value: OriginalFormatInfo, binding: WorkflowBinding): Unit {
    if (value.binding != binding || value.sourceAssetId.length != 64 || value.sourceAssetId.any { it !in "0123456789abcdef" } ||
        value.format !in setOf("png", "jpeg", "webp", "heic", "heif", "avif", "bmp", "tiff") ||
        value.sourceBitDepth.toInt() !in listOf(8, 16) || (value.firstTiffPageOnly && value.format != "tiff"))
        throw IllegalArgumentException("Original format receipt does not match its owned request")
}
public fun originalFormatNotice(value: OriginalFormatInfo): String? = if (value.firstTiffPageOnly)
    "Only the first TIFF page is displayed and exported. All original pages remain in the unchanged source file." else null

/** Full revision check after the worker returns, before a platform adopts a notice. */
public fun originalFormatRevisionMatches(info: ProjectInfo, binding: WorkflowBinding): Boolean =
    info.projectId == binding.projectId && info.hostSeq == binding.hostSeq && info.stateHash == binding.stateHash &&
        binding.documentId in info.documentIds
