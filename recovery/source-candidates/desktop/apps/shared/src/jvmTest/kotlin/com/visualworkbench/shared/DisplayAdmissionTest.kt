package com.visualworkbench.shared

import org.junit.Assert.*
import org.junit.Test

class DisplayAdmissionTest {
    private val binding = WorkflowBinding("00000000-0000-7000-8000-000000000001", "00000000-0000-7000-8000-000000000002", 7uL, "a".repeat(64))
    private fun request() = DisplayColorRequest(binding, DisplayRegion(0u, 0u, 2u, 1u), DisplayColorSpace.DisplayP3)
    private fun pixels() = DisplayColorPixels(binding, "b".repeat(64), DisplayRegion(0u, 0u, 2u, 1u), DisplayColorSpace.DisplayP3,
        byteArrayOf(255.toByte(), 0, 0, 255.toByte(), 0, 255.toByte(), 0, 127), ByteArray(132), 8u, false, false, 52uL * 1024uL * 1024uL)
    private fun refuses(value: DisplayColorPixels, request: DisplayColorRequest = request()) {
        try { validateDisplayPixels(value, request); fail("Expected response refusal") }
        catch (error: DisplayColorFailure) { assertEquals(DisplayColorFailureKind.Invalid, error.kind) }
    }
    @Test fun exactRequestIsAdmittedWithoutMutatingBytes() { val p = pixels(); val old = p.rgba.clone(); validateDisplayPixels(p, request()); assertArrayEquals(old, p.rgba) }
    @Test fun sameHostSequenceDifferentHashIsRefused() { refuses(pixels().copy(binding = binding.copy(stateHash = "c".repeat(64)))) }
    @Test fun sourceIdentityMustBeCanonicalHash() { refuses(pixels().copy(sourceAssetId = "G".repeat(64))) }
    @Test fun targetSpaceAndExactRegionAreBoundToTheRequest() { refuses(pixels().copy(space = DisplayColorSpace.Srgb)); refuses(pixels().copy(region = DisplayRegion(1u, 0u, 2u, 1u))) }
    @Test fun malformedAndOversizedPixelBuffersAreRefused() { refuses(pixels().copy(rgba = byteArrayOf(0))); refuses(pixels().copy(region = DisplayRegion(0u, 0u, UInt.MAX_VALUE, 2u))) }
    @Test fun responseCannotGrantItsOwnUntaggedOrDepthConsent() {
        refuses(pixels().copy(assumedSrgb = true)); refuses(pixels().copy(sourceBitDepth = 16u, depthReduced = true))
        refuses(pixels().copy(sourceBitDepth = 16u, depthReduced = false))
        validateDisplayPixels(pixels().copy(sourceBitDepth = 16u, depthReduced = true, assumedSrgb = true), request().copy(allowDisplayDepthReduction = true, assumeUntaggedSrgb = true))
    }
    @Test fun reportedMemoryAndProfilesMustFitTheOwnedBudget() {
        refuses(pixels().copy(estimatedPeakBytes = 0uL)); refuses(pixels().copy(estimatedPeakBytes = request().memoryBudgetBytes + 1uL))
        refuses(pixels().copy(iccProfile = ByteArray(4 * 1024 * 1024 + 1)))
    }
    @Test fun laterRequestRetiresAnEarlierAdoption() { val gate = DisplayAdoption(); val first = gate.issue(3, request()); val last = gate.issue(3, request()); assertFalse(gate.accepts(first, 3, binding, DisplayColorSpace.DisplayP3)); assertTrue(gate.accepts(last, 3, binding, DisplayColorSpace.DisplayP3)) }
    @Test fun epochRevisionAndWindowChoiceIndependentlyRetireAnAdoption() {
        val gate = DisplayAdoption(); val ticket = gate.issue(3, request())
        assertFalse(gate.accepts(ticket, 4, binding, DisplayColorSpace.DisplayP3))
        assertFalse(gate.accepts(ticket, 3, binding.copy(stateHash = "c".repeat(64)), DisplayColorSpace.DisplayP3))
        assertFalse(gate.accepts(ticket, 3, binding, DisplayColorSpace.Srgb))
        assertFalse(gate.accepts(ticket, 3, null, DisplayColorSpace.DisplayP3))
    }
    @Test fun explicitInvalidationRetiresACompletedButUnadoptedImage() { val gate = DisplayAdoption(); val ticket = gate.issue(3, request()); gate.invalidate(); assertFalse(gate.accepts(ticket, 3, binding, DisplayColorSpace.DisplayP3)) }
}
