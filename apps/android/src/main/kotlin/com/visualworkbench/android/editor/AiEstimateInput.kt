package com.visualworkbench.android.editor

import com.visualworkbench.shared.AiTokenEstimate
import java.time.LocalDate
import java.time.temporal.ChronoUnit

/** Explicit dated provider estimates only. The app has no invented image-token
 * formula and never fills missing figures from the requested image dimensions. */
internal fun aiEstimateInput(text: String, input: String, output: String,
    provenance: String, verified: String, expires: String): AiTokenEstimate? {
    val fields = listOf(text, input, output, provenance, verified, expires)
    if (fields.all(String::isEmpty)) return null
    fun count(value: String, positive: Boolean): ULong {
        if (value.isEmpty() || value.length > 9 || value.any { it !in '0'..'9' })
            throw IllegalArgumentException("Enter decimal token counts up to 100000000.")
        val number = value.toULong()
        if (number > 100_000_000uL || (positive && number == 0uL))
            throw IllegalArgumentException("Image token estimates must be positive and at most 100000000.")
        return number
    }
    if (provenance.isBlank() || provenance.length > 1024 || provenance.encodeToByteArray().size > 1024 ||
        provenance.any(Char::isISOControl)) throw IllegalArgumentException("Give the estimate's source, up to 1024 UTF-8 bytes.")
    fun date(value: String): LocalDate {
        if (!Regex("[0-9]{4}-[0-9]{2}-[0-9]{2}").matches(value))
            throw IllegalArgumentException("Use dates in YYYY-MM-DD form.")
        return try { LocalDate.parse(value) } catch (_: Exception) {
            throw IllegalArgumentException("Enter valid calendar dates.")
        }
    }
    val first = date(verified); val last = date(expires)
    if (ChronoUnit.DAYS.between(first, last) !in 0L..31L)
        throw IllegalArgumentException("The estimate can cover at most 31 days.")
    return AiTokenEstimate(textInput = count(text, false), imageInput = count(input, true),
        imageOutput = count(output, true), provenance = provenance, verifiedOn = verified, expiresOn = expires)
}
