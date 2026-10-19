package com.visualworkbench.shared

import kotlin.test.Test
import kotlin.test.assertFalse

class GiantEditingCapabilityTest {
    @Test
    fun rawTileFakeHasNoImplicitCompositionCapability() {
        val raw: GiantImage = object : GiantImage {
            override suspend fun viewport(view: GiantViewport): GiantViewportPlan =
                error("unused")
            override suspend fun tile(
                address: GiantTileAddress,
                foreign: SessionResourceCharge,
            ): GiantTilePixels = error("unused")
            override suspend fun close() = Unit
        }
        assertFalse(raw is GiantEditingImage)
    }
}
