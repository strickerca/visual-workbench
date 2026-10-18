package com.visualworkbench.shared.assistedselection.backend

import com.visualworkbench.shared.*
import kotlinx.coroutines.ensureActive
import kotlin.coroutines.CoroutineContext
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

internal data class SamSize(val width: Int, val height: Int)
internal fun samSize(width: UInt, height: UInt): SamSize {
    if (width !in 1u..32768u || height !in 1u..32768u) unsupported()
    val scale=1024.0/max(width,height).toDouble()
    val result=SamSize(floor(width.toDouble()*scale+0.5).toInt(),floor(height.toDouble()*scale+0.5).toInt())
    // Upstream ResizeLongestSide rejects a zero rounded short side. Do not
    // silently change its shape policy or substitute a display thumbnail.
    if (result.width !in 1..1024 || result.height !in 1..1024) unsupported()
    return result
}

private data class Coefficients(val first: Int, val weights: IntArray)
private fun pillowCoefficients(input: Int, output: Int): List<Coefficients> {
    val scale=input.toDouble()/output; val filter=max(1.0,scale)
    return List(output) { pixel ->
        val center=(pixel+0.5)*scale
        val first=max(0,(center-filter+0.5).toInt())
        val end=min(input,(center+filter+0.5).toInt())
        val values=DoubleArray(end-first) { max(0.0,1.0-abs((it+first-center+0.5)/filter)) }
        val sum=values.sum(); if (sum <= 0.0) corrupt()
        Coefficients(first,IntArray(values.size) { floor(values[it]/sum*(1 shl 22)+0.5).toInt() })
    }
}
private fun clipPillow(value: Long): Int = (value shr 22).coerceIn(0,255).toInt()

/** Pillow's separable antialiased bilinear RGB8 resize uses 22-bit normalized
 * coefficients and rounds to uint8 after EACH axis. Normalize only afterward,
 * with upstream RGB means/stds, then pad normalized zeros on right/bottom.
 * Alpha is not a SAM input; RGB samples are consumed without editing originals. */
internal fun samImage(rgba: ByteArray, width: UInt, height: UInt, task: CoroutineContext): FloatArray {
    val target=samSize(width,height)
    if (rgba.size.toULong() != width.toULong()*height.toULong()*4uL) corrupt()
    val xCoefficients=pillowCoefficients(width.toInt(),target.width)
    val yCoefficients=pillowCoefficients(height.toInt(),target.height)
    val rows=mutableMapOf<Int,ByteArray>()
    val result=FloatArray(IMAGE_ELEMENTS)
    val means=floatArrayOf(123.675f,116.28f,103.53f); val std=floatArrayOf(58.395f,57.12f,57.375f)
    yCoefficients.forEachIndexed { y,vertical ->
        task.ensureActive()
        rows.keys.filter { it < vertical.first }.forEach(rows::remove)
        for (sourceY in vertical.first until vertical.first+vertical.weights.size) if (sourceY !in rows) {
            val row=ByteArray(target.width*3)
            xCoefficients.forEachIndexed { x,horizontal ->
                for (channel in 0..2) {
                    var sum=1L shl 21
                    horizontal.weights.forEachIndexed { index,weight ->
                        val offset=((sourceY.toLong()*width.toLong()+horizontal.first+index)*4+channel).toInt()
                        sum+=(rgba[offset].toInt() and 255).toLong()*weight
                    }
                    row[x*3+channel]=clipPillow(sum).toByte()
                }
            }
            rows[sourceY]=row
        }
        for (x in 0 until target.width) for (channel in 0..2) {
            var sum=1L shl 21
            vertical.weights.forEachIndexed { index,weight ->
                sum+=(rows.getValue(vertical.first+index)[x*3+channel].toInt() and 255).toLong()*weight
            }
            result[channel*1024*1024+y*1024+x]=(clipPillow(sum).toFloat()-means[channel])/std[channel]
        }
    }
    return result
}

internal data class SamPrompt(val coordinates: FloatArray, val labels: IntArray, val valid: IntArray, val count: IntArray)
internal fun samPrompt(request: AssistedSelectionPromptRequest): SamPrompt {
    val source=request.source; val prompt=request.prompt.owned(source); val size=samSize(source.document.width,source.document.height)
    val coordinates=FloatArray(66*2); val labels=IntArray(66) { -1 }; val valid=IntArray(66); var count=0
    fun point(x: Double, y: Double, label: Int) {
        if (count >= 66) limit()
        coordinates[count*2]=(x*size.width/source.document.width.toDouble()).toFloat()
        coordinates[count*2+1]=(y*size.height/source.document.height.toDouble()).toFloat()
        labels[count]=label; valid[count]=1; count++
    }
    prompt.positive.forEach { point(it.x,it.y,1) }; prompt.negative.forEach { point(it.x,it.y,0) }
    prompt.box?.let { point(it.x,it.y,2); point(it.x+it.width,it.y+it.height,3) }
        ?: run { valid[count]=1; count++ } // Exactly one valid not-a-point sentinel, only without a box.
    return SamPrompt(coordinates,labels,valid,intArrayOf(count))
}

private data class Sample(val first: Int, val second: Int, val weight: Float)
private fun interpolation(input: Int, output: Int): List<Sample> = List(output) { pixel ->
    val coordinate=max(0.0,(pixel+0.5)*input.toDouble()/output-0.5)
    val first=min(input-1,floor(coordinate).toInt())
    Sample(first,min(first+1,input-1),(coordinate-first).toFloat())
}
private fun blend(a: Float, b: Float, weight: Float): Float = a*(1f-weight)+b*weight

/** SAM mask0 raw logits: bilinear(256->1024, align_corners=false), crop the
 * right/bottom padding, bilinear to full oriented D, then strictly >0. No sigmoid,
 * score-based mask switching, thumbnail coverage or prompt retry is introduced. */
internal fun samCoverage(logits: FloatArray, score: FloatArray, width: UInt, height: UInt,
    task: CoroutineContext): ByteArray {
    if (logits.size != LOGIT_ELEMENTS || logits.any { !it.isFinite() } || score.size != 1 || !score[0].isFinite()) corrupt()
    val size=samSize(width,height); val expanded=FloatArray(1024*1024)
    val up=interpolation(256,1024)
    up.forEachIndexed { y,sy ->
        task.ensureActive()
        up.forEachIndexed { x,sx ->
            expanded[y*1024+x]=blend(blend(logits[sy.first*256+sx.first],logits[sy.first*256+sx.second],sx.weight),
                blend(logits[sy.second*256+sx.first],logits[sy.second*256+sx.second],sx.weight),sy.weight)
        }
    }
    val outWidth=width.toInt(); val outHeight=height.toInt()
    val pixels=width.toULong()*height.toULong(); if (pixels > 50_000_000uL) limit()
    val coverage=ByteArray(pixels.toInt()); val horizontal=interpolation(size.width,outWidth)
    interpolation(size.height,outHeight).forEachIndexed { y,sy ->
        task.ensureActive()
        horizontal.forEachIndexed { x,sx ->
            val value=blend(blend(expanded[sy.first*1024+sx.first],expanded[sy.first*1024+sx.second],sx.weight),
                blend(expanded[sy.second*1024+sx.first],expanded[sy.second*1024+sx.second],sx.weight),sy.weight)
            coverage[y*outWidth+x]=if (value > 0f) 255.toByte() else 0
        }
    }
    return coverage
}
