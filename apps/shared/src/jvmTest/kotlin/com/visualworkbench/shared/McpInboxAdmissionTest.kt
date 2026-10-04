package com.visualworkbench.shared
import org.junit.Assert.*
import org.junit.Test
class McpInboxAdmissionTest {
    private fun input()=McpInboxSubmission("0".repeat(36),"0".repeat(36),"generic","a".repeat(64),"b".repeat(32),1,png=byteArrayOf(1,2,3),note="literal note")
    @Test fun returnedInputOwnsItsEncodedBytesBeforeSuspension(){val original=input();val copy=ownedMcpSubmission(original);original.png!![0]=9;assertArrayEquals(byteArrayOf(1,2,3),copy.png)}
    @Test fun textAndImageTogetherOrNoBodyNeverReachNativeOwnership(){for(value in listOf(input().copy(text="both"),input().copy(png=null))){try{ownedMcpSubmission(value);fail("invalid body admitted")}catch(e:PackageFailure){assertEquals(PackageFailureKind.Invalid,e.kind)}}}
    @Test fun utf8TextAndEncodedImageBoundsAreActualByteLimits(){for(value in listOf(input().copy(note="界".repeat(12000)),input().copy(png=ByteArray(4*1024*1024+1)))){try{ownedMcpSubmission(value);fail("oversized input admitted")}catch(e:PackageFailure){assertEquals(PackageFailureKind.Limit,e.kind)}}}
}
