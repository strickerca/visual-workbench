package com.visualworkbench.shared

import com.visualworkbench.bindings.core.AvifImageException
import com.visualworkbench.bindings.core.AvifImageInfo
import com.visualworkbench.bindings.core.AvifImageRequest
import com.visualworkbench.bindings.core.AvifColorInfo
import org.junit.Assert.*
import org.junit.Test

/** These cases exercise actual pre-OS admission only; no installed codec or
 * valid AV1 tile fixture is implied. Parent must run platform fidelity cases. */
class AvifAdmissionInstrumentedTest {
    private fun request()=AvifImageRequest(ByteArray(64),AvifImageInfo("a".repeat(64),16u,8u,8u,1u,byteArrayOf(),AvifColorInfo(1u,13u,6u,true),128uL*128uL*64uL+256uL+32uL*1024uL*1024uL),256uL*1024uL*1024uL)
    private inline fun <reified T:Throwable> refuses(block:()->Unit) {
        try { block(); fail("Expected typed refusal") } catch(error:Throwable) { if(error !is T)throw error }
    }
    @Test fun admittedResourceEnvelopeNeedsNoOsRead() {admitAndroidAvif(request())}
    @Test fun callerCannotUnderstatePeak() {val r=request();refuses<AvifImageException.Memory>{admitAndroidAvif(r.copy(info=r.info.copy(estimatedPeakBytes=1uL)))}}
    @Test fun rawIccAndHigherDepthDoNotBecomeSrgbEightBit() {val r=request();refuses<AvifImageException.Color>{admitAndroidAvif(r.copy(info=r.info.copy(iccProfile=byteArrayOf(1))))};refuses<AvifImageException.Depth>{admitAndroidAvif(r.copy(info=r.info.copy(bitDepth=10u)))}}
    @Test fun orientationAndTinyBudgetRefuseBeforeCodecProbe() {val r=request();refuses<AvifImageException.Orientation>{admitAndroidAvif(r.copy(info=r.info.copy(orientation=6u)))};refuses<AvifImageException.Memory>{admitAndroidAvif(r.copy(memoryBudgetBytes=1024uL))} }
    @Test fun missingOrHdrColorDoesNotReachCodecProbe() {val r=request();refuses<AvifImageException.Color>{admitAndroidAvif(r.copy(info=r.info.copy(nclx=null)))};refuses<AvifImageException.Color>{admitAndroidAvif(r.copy(info=r.info.copy(nclx=AvifColorInfo(9u,16u,9u,true))))}}
}
