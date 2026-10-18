package com.visualworkbench.shared.assistedselection.backend

import com.visualworkbench.shared.*
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test

class LiteRtSamTensorsTest {
    @Test fun roundedResizeUsesActualAxesAndRefusesZeroShortSideInsteadOfClamping() {
        assertEquals(SamSize(1024,683),samSize(300u,200u))
        assertEquals(SamSize(683,1024),samSize(200u,300u))
        assertEquals(SamSize(1024,1024),samSize(1u,1u))
        fails(SelectionFailureKind.Unsupported) { samSize(32768u,1u) }
        fails(SelectionFailureKind.Unsupported) { samSize(0u,1u) }
        fails(SelectionFailureKind.Unsupported) { samSize(32769u,32769u) }
    }

    @Test fun pointsUseRoundedAxisRatiosAndOneValidSentinelWithEveryUnusedSlotMasked() {
        val source=source(300u,200u)
        val prompt=samPrompt(AssistedSelectionPromptRequest(source,1uL,
            AssistedSelectionPrompt(listOf(Point(100.0,100.0)),listOf(Point(20.0,30.0)))))
        assertEquals(3,prompt.count.single())
        assertEquals(100f*1024/300,prompt.coordinates[0],0.0001f)
        assertEquals(100f*683/200,prompt.coordinates[1],0.0001f)
        assertArrayEquals(intArrayOf(1,0,-1),prompt.labels.copyOf(3))
        assertArrayEquals(intArrayOf(1,1,1),prompt.valid.copyOf(3))
        assertEquals(1,prompt.labels.indices.count { prompt.labels[it] == -1 && prompt.valid[it] == 1 })
        assertTrue((3 until 66).all { prompt.valid[it] == 0 && prompt.labels[it] == -1 })
        assertTrue(prompt.coordinates.copyOfRange(4,132).all { it == 0f })
    }

    @Test fun boxCornersUseLabelsTwoAndThreeAndNeverAddAValidSentinel() {
        val source=source(300u,200u)
        val prompt=samPrompt(AssistedSelectionPromptRequest(source,2uL,
            AssistedSelectionPrompt(box=Rect(0.0,0.0,300.0,200.0))))
        assertEquals(2,prompt.count.single()); assertArrayEquals(intArrayOf(2,3),prompt.labels.copyOf(2))
        assertEquals(1024f,prompt.coordinates[2],0f); assertEquals(683f,prompt.coordinates[3],0f)
        assertEquals(0,prompt.labels.indices.count { prompt.labels[it] == -1 && prompt.valid[it] == 1 })
        assertTrue((2 until 66).all { prompt.valid[it] == 0 })
    }

    @Test fun maximumPromptFitsExactlyAndExcessOrInvalidGeometryIsRefused() {
        val source=source(64u,64u)
        val points=List(64) { Point(it.toDouble(),1.0) }
        val prompt=samPrompt(AssistedSelectionPromptRequest(source,1uL,
            AssistedSelectionPrompt(points,box=Rect(0.0,0.0,64.0,64.0))))
        assertEquals(66,prompt.count.single()); assertTrue(prompt.valid.all { it == 1 })
        assertArrayEquals(intArrayOf(2,3),prompt.labels.copyOfRange(64,66))
        fails(SelectionFailureKind.Limit) { samPrompt(AssistedSelectionPromptRequest(source,1uL,AssistedSelectionPrompt(points+Point(0.0,0.0)))) }
        fails(SelectionFailureKind.Invalid) { samPrompt(AssistedSelectionPromptRequest(source,1uL,AssistedSelectionPrompt(listOf(Point(64.0,0.0))))) }
        fails(SelectionFailureKind.Limit) { samPrompt(AssistedSelectionPromptRequest(source,1uL,AssistedSelectionPrompt())) }
    }

    @Test fun rgbNormalizationIsOnceAlphaIsDroppedAndPaddingIsNormalizedZero() {
        val input=ByteArray(2*4)
        input[0]=255.toByte(); input[3]=0; input[4]=255.toByte(); input[7]=255.toByte()
        val tensor=samImage(input,2u,1u,kotlin.coroutines.EmptyCoroutineContext)
        assertEquals(IMAGE_ELEMENTS,tensor.size)
        assertEquals((255f-123.675f)/58.395f,tensor[0],0f)
        assertEquals(tensor[0],tensor[1023],0f) // Same RGB despite distinct alpha.
        assertEquals((0f-116.28f)/57.12f,tensor[1024*1024],0f)
        assertEquals(0f,tensor[512*1024],0f)
        assertEquals(0f,tensor[1024*1024+512*1024],0f)
        assertEquals(255.toByte(),input[0]); assertEquals(0.toByte(),input[3])
    }

    @Test fun syntheticBilinearSamplesMatchInstalledPillow1230GoldenAfterRgb8AxisRounding() {
        // Generated without models/inference using Pillow12.3.0 RGB BILINEAR.
        val means=floatArrayOf(123.675f,116.28f,103.53f); val std=floatArrayOf(58.395f,57.12f,57.375f)
        for ((width,height,samples) in listOf(
            Triple(3,2,listOf(Triple(0,0,listOf(0,53,106)),Triple(1023,0,listOf(74,127,180)),
                Triple(512,341,listOf(83,136,189)),Triple(1023,682,listOf(165,218,15)))),
            Triple(1025,2,listOf(Triple(0,0,listOf(0,53,106)),Triple(1023,0,listOf(0,53,106)),
                Triple(512,1,listOf(110,163,216)),Triple(1023,1,listOf(91,144,197)))))) {
            val rgba=ByteArray(width*height*4)
            for (y in 0 until height) for (x in 0 until width) for (c in 0..2)
                rgba[(y*width+x)*4+c]=((x*37+y*91+c*53)%256).toByte()
            val tensor=samImage(rgba,width.toUInt(),height.toUInt(),kotlin.coroutines.EmptyCoroutineContext)
            for ((x,y,channels) in samples) for (c in 0..2)
                assertEquals((channels[c].toFloat()-means[c])/std[c],tensor[c*1024*1024+y*1024+x],0.000001f)
        }
    }

    @Test fun coverageUsesMaskZeroStrictThresholdAndRetainsTheFullOriginalGrid() {
        val positive=samCoverage(FloatArray(LOGIT_ELEMENTS) { 0.01f },floatArrayOf(-100f),300u,200u,kotlin.coroutines.EmptyCoroutineContext)
        assertEquals(60000,positive.size); assertTrue(positive.all { it == 255.toByte() })
        val zero=samCoverage(FloatArray(LOGIT_ELEMENTS),floatArrayOf(1f),3u,2u,kotlin.coroutines.EmptyCoroutineContext)
        assertTrue(zero.all { it == 0.toByte() })
        val negative=samCoverage(FloatArray(LOGIT_ELEMENTS) { -0.01f },floatArrayOf(999f),3u,2u,kotlin.coroutines.EmptyCoroutineContext)
        assertTrue(negative.all { it == 0.toByte() })
    }

    @Test fun inverseUnpadsBeforeSecondInterpolationInsteadOfStretchingPaddedRowsIntoCoverage() {
        val logits=FloatArray(LOGIT_ELEMENTS) { index -> if (index/256 < 128) 1f else -1f }
        val coverage=samCoverage(logits,floatArrayOf(0f),16u,8u,kotlin.coroutines.EmptyCoroutineContext)
        // Actual resized content is only the top512 rows of1024; bottom padding
        // logits must not occupy half the restored original image.
        assertTrue(coverage.all { it == 255.toByte() })
    }

    @Test fun nonfiniteOrWrongSizedOutputsAndCancelledRowWorkAreRefused() {
        fails(SelectionFailureKind.Corrupt) { samCoverage(FloatArray(1),floatArrayOf(1f),3u,2u,kotlin.coroutines.EmptyCoroutineContext) }
        val bad=FloatArray(LOGIT_ELEMENTS); bad[0]=Float.NaN
        fails(SelectionFailureKind.Corrupt) { samCoverage(bad,floatArrayOf(1f),3u,2u,kotlin.coroutines.EmptyCoroutineContext) }
        fails(SelectionFailureKind.Corrupt) { samCoverage(FloatArray(LOGIT_ELEMENTS),floatArrayOf(Float.POSITIVE_INFINITY),3u,2u,kotlin.coroutines.EmptyCoroutineContext) }
        val cancelled=Job().also { it.cancel() }
        try { samImage(ByteArray(16),2u,2u,cancelled); fail("Cancelled resize admitted") } catch (_: CancellationException) { }
        try { samCoverage(FloatArray(LOGIT_ELEMENTS),floatArrayOf(1f),3u,2u,cancelled); fail("Cancelled inverse admitted") } catch (_: CancellationException) { }
    }

    private fun fails(kind: SelectionFailureKind, block: () -> Unit) {
        try { block(); fail("Expected $kind") } catch (failure: SelectionFailure) { assertEquals(kind,failure.kind) }
    }
    private fun source(width: UInt, height: UInt): AssistedSelectionSource = backendSource(width,height)
}

internal fun backendSource(width: UInt = 4u, height: UInt = 4u): AssistedSelectionSource {
    val project="00000000-0000-4000-8000-000000000001"; val document="00000000-0000-4000-8000-000000000002"
    val revision=ProjectInfo(project,"Test","00000000-0000-4000-8000-000000000003",1uL,false,false,0uL,"a".repeat(64),listOf(document))
    return AssistedSelectionSource(SelectionDocument(SelectionBinding(project,document,"b".repeat(64),revision.hostSeq,revision.stateHash),
        width,height,8u,revision),null,1)
}
