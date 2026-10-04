package com.visualworkbench.shared

import com.visualworkbench.bindings.core.OsImageException
import com.visualworkbench.bindings.core.OsImageInfo as NativeInfo
import com.visualworkbench.bindings.core.OsImageRequest
import org.junit.Assert.*
import org.junit.Test

/** Admission only: no valid image or OS codec call is reached by these cases. */
class OsImageAdmissionInstrumentedTest {
    private fun info()=NativeInfo("0".repeat(64),16u,8u,8u.toUByte(),1u.toUByte(),byteArrayOf(),64uL*1024uL*1024uL)
    private fun refusal(info:NativeInfo,budget:ULong=256uL*1024uL*1024uL):OsImageException{
        try{AndroidOsImages.decode(OsImageRequest(byteArrayOf(1,2,3),info,budget));error("Unadmitted image reached codec")}
        catch(value:OsImageException){return value}
    }
    @Test fun highDepthIsNeverQuietlyConvertedToEightBit(){assertTrue(refusal(info().copy(bitDepth=10u.toUByte())) is OsImageException.Depth)}
    @Test fun rotatedSourceNeedsExplicitAdapter(){assertTrue(refusal(info().copy(orientation=6u.toUByte())) is OsImageException.Orientation)}
    @Test fun arbitraryOriginalIccIsNotReplacedWithSrgb(){assertTrue(refusal(info().copy(iccProfile=byteArrayOf(1))) is OsImageException.Color)}
    @Test fun callerBudgetIsCheckedBeforeAnyCodecWork(){assertTrue(refusal(info(),1024uL) is OsImageException.Memory)}
}
