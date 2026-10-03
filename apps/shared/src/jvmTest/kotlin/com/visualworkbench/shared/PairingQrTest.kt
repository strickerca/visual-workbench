package com.visualworkbench.shared

import org.junit.Test
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertThrows

class PairingQrTest {
    @Test fun binaryPayloadRoundTripsWithoutParsingOrTextLoss() {
        val bytes=ByteArray(4096) { it.toByte() }
        assertArrayEquals(bytes,decodePairingQr(encodePairingQr(bytes)))
    }
    @Test fun rejectsUntaggedOversizePaddingAndNoncanonicalLastBits() {
        for(text in listOf("{}","vw-pair:v1:","vw-pair:v1:YQ==","vw-pair:v1:YR","vw-pair:v1:"+"A".repeat(5463),"vw-pair:v1:Y Q")) {
            assertThrows(SessionFailure::class.java) { decodePairingQr(text) }
        }
        assertThrows(SessionFailure::class.java) { encodePairingQr(ByteArray(4097)) }
    }
}
