package com.visualworkbench.shared

/** No method other than send starts a provider request. Settings key writes are
 * separate explicit owner actions. All suspending operations settle their
 * owned native producer on cancellation before returning. */
public enum class AiFailureKind { Invalid, Stale, Limit, Depth, Color, Estimate, SoftBudget, Unresolved,
    AttemptConsumed, Credentials, Provider, Proof, Storage, Unsupported, Cancelled, Closed, Busy }
public class AiFailure(public val kind: AiFailureKind) : Exception("AI edit: ${kind.name}")
public data class AiConfiguration(public val json: String, public val fingerprint: String,
    public val verifiedOn: String, public val expiresOn: String, public val model: String,
    public val quality: String, public val dailySoftBudgetMicrousd: ULong)
public data class AiTokenEstimate(public val schema: UInt = 1u, public val textInput: ULong,
    public val imageInput: ULong, public val imageOutput: ULong, public val provenance: String,
    public val verifiedOn: String, public val expiresOn: String)
public data class AiPrepareOptions(public val binding: WorkflowBinding,
    public val selections: List<SelectionVersion>, public val intentId: String,
    public val instruction: String, public val configurationFingerprint: String,
    public val estimate: AiTokenEstimate?, public val featherPx: UInt = 8u,
    public val allow16bitProviderCopy: Boolean = false, public val assumeUntaggedSrgb: Boolean = false,
    public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL)
public data class AiReview(public val requestId: String, public val binding: WorkflowBinding,
    public val sourceAssetId: String, public val sourceFileSha256: String, public val maskSha256: String,
    public val configurationFingerprint: String, public val model: String, public val quality: String,
    public val sourceWidth: UInt, public val sourceHeight: UInt, public val sourceBitDepth: UByte,
    public val modelWidth: UInt, public val modelHeight: UInt, public val cropX: Int, public val cropY: Int,
    public val cropWidth: UInt, public val cropHeight: UInt, public val featherPx: UInt,
    public val estimatedMicrousd: ULong?, public val estimateProvenance: String?,
    public val estimateExpiresOn: String?, public val configurationExpiresOn: String,
    public val providerCopyReducesDepth: Boolean)
public data class AiReadiness(public val configured: Boolean, public val trustReady: Boolean,
    public val configurationCurrent: Boolean, public val spentTodayMicrousd: ULong,
    public val dailySoftBudgetMicrousd: ULong)
public enum class AiSettlement { UsagePriced, UsageMissing, LedgerUnavailable }
public data class AiProofInfo(public val changedOutside: ULong, public val outsideSha256Before: String,
    public val outsideSha256After: String, public val changedUnaccepted: ULong?,
    public val unacceptedSha256Before: String?, public val unacceptedSha256After: String?,
    public val deltaE2000Mean: Double, public val deltaE2000Max: Double, public val ssimInsideMask: Double)
public data class AiCandidateInfo(public val requestId: String, public val candidateId: String,
    public val binding: WorkflowBinding, public val width: UInt, public val height: UInt,
    public val bitDepth: UByte, public val partial: Boolean, public val settlement: AiSettlement,
    public val actualMicrousd: ULong?, public val proof: AiProofInfo)
public sealed interface AiCompareMode {
    public data object Before : AiCompareMode
    public data object After : AiCompareMode
    public data class Wipe(public val vertical: Boolean, public val cut: UInt, public val resultBefore: Boolean = true) : AiCompareMode
    public data object Split : AiCompareMode
    public data class Difference(public val gain: UByte = 1u) : AiCompareMode
}
/** Full-resolution rectangle. Maximum 512x512, no scaling. Split coordinates
 * address the native side-by-side image, whose width is twice the source. */
public data class AiRegion(public val x: UInt, public val y: UInt, public val width: UInt, public val height: UInt)
public data class AiPixels(public val candidateId: String, public val region: AiRegion,
    public val canvasWidth: UInt, public val canvasHeight: UInt, public val rgbaSrgb: ByteArray)
public data class AiAcceptanceBrush(public val expectedCandidateId: String, public val nextCandidateId: String,
    public val points: List<Point>, public val radius: Double, public val opacity: UByte = 255u,
    public val subtract: Boolean = false, public val clearFirst: Boolean = false)
public data class AiSaveOptions(public val expected: WorkflowBinding, public val metadata: WorkflowMetadata,
    public val resultId: String, public val layerId: String, public val objectId: String,
    public val replaceObjectId: String? = null)
public data class AiResultReceipt(public val transactionId: String, public val resultId: String,
    public val objectId: String, public val layerId: String, public val outputAssetId: String,
    public val compositeAssetId: String, public val acceptanceMaskAssetId: String?,
    public val revision: ProjectInfo, public val proof: AiProofInfo)
public data class AiResultPixels(public val binding: WorkflowBinding, public val resultId: String,
    public val sourceAssetId: String, public val compositeAssetId: String, public val region: AiRegion,
    public val canvasWidth: UInt, public val canvasHeight: UInt, public val rgbaSrgb: ByteArray)

public data class AiInstructionSummary(public val id:String,public val role:String,public val text:String,
    public val targetObjectIds:List<String>,public val markerNumbers:List<UInt>)
public data class AiContext(public val binding:WorkflowBinding,public val sourceAssetId:String,
    public val width:UInt,public val height:UInt,public val bitDepth:UInt,
    public val changeSelections:List<SelectionVersion>,public val instructions:List<AiInstructionSummary>)
public interface WorkbenchAiService {
    public suspend fun configuration(): AiConfiguration
    public suspend fun configure(json: String, expectedFingerprint: String): AiConfiguration
    public suspend fun configureDailyBudget(expectedFingerprint:String,dailySoftBudgetMicrousd:ULong):AiConfiguration
    public suspend fun readiness(): AiReadiness
    /** Windows only; input is consumed/cleared by the adapter. Android settings
     * use the existing secure ProviderKeyDialog/Keystore owner. Never pass chat keys. */
    public suspend fun saveWindowsKey(bytes: ByteArray)
    public suspend fun removeWindowsKey()
    public suspend fun close()
}
public interface WorkbenchAiResults {
    public suspend fun context(binding:WorkflowBinding,memoryBudgetBytes:ULong=256uL*1024uL*1024uL):AiContext
    public suspend fun resultPixels(binding: WorkflowBinding, resultId: String, region: AiRegion,
        memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL): AiResultPixels
    public suspend fun resultStatus(binding: WorkflowBinding, resultId: String, accepted: Boolean,
        metadata: WorkflowMetadata): WorkflowReceipt
}
public interface WorkbenchAiEdits : WorkbenchAiResults {
    public suspend fun prepare(options:AiPrepareOptions):WorkbenchAiRequest
}
public interface WorkbenchAiRequest {
    public val review: AiReview
    /** Call solely from explicit Send on this displayed immutable review.
     * This is an estimate, not a provider-enforced charge cap. No automatic retry. */
    public suspend fun send(requestId: String, displayedEstimateMicrousd: ULong,
        acknowledgeSoftBudget: Boolean): AiCandidateInfo
    public suspend fun candidate(): AiCandidateInfo
    public suspend fun compare(candidateId: String, mode: AiCompareMode, region: AiRegion): AiPixels
    /** Candidate-local mask edit; no canonical document edit or revision change.
     * It derives from immutable source/base output and proves unaccepted bytes. */
    public suspend fun acceptBrush(brush: AiAcceptanceBrush): AiCandidateInfo
    public suspend fun save(candidateId: String, options: AiSaveOptions): AiResultReceipt
    public suspend fun close()
}
public expect fun defaultAiConfiguration(): AiConfiguration
public expect fun validateAiConfiguration(json: String): AiConfiguration
public expect suspend fun createAiService(applicationPrivateDirectory: String,
    provisionFirstInstall: Boolean = false): WorkbenchAiService
public expect fun WorkbenchProject.aiEdits(service: WorkbenchAiService): WorkbenchAiEdits

/** Read/review saved project Results without provisioning AI history or keys. */
public expect fun WorkbenchProject.aiResults():WorkbenchAiResults
