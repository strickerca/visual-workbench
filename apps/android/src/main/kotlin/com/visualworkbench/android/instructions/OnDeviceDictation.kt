package com.visualworkbench.android.instructions

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import androidx.annotation.RequiresApi
import com.visualworkbench.shared.instructionTextFits
import java.util.Locale

/** No default/cloud recognizer fallback. One explicit microphone session, no
 * saved audio, a 60s deadline and generation-fenced callbacks on Main. */
internal class OnDeviceDictation(
    private val context: Context,
    private val result: (String, Boolean) -> Unit,
    private val status: (String) -> Unit,
    private val factory: DictationFactory = PlatformDictationFactory,
) : AutoCloseable {
    private val handler = Handler(Looper.getMainLooper())
    private var engine: DictationEngine? = null
    private var generation = 0L
    private var closed = false
    private val deadline = Runnable { stop(); status("Dictation stopped after 60 seconds. Review the text or start another session.") }
    fun available(): Boolean = !closed && factory.available(context)
    fun start(): Boolean {
        check(Looper.myLooper() == Looper.getMainLooper())
        stop()
        if (closed) return false
        if (!factory.available(context)) { status("On-device dictation is unavailable. Install or enable an on-device speech provider in Android settings, or use the keyboard."); return false }
        if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) { status("Microphone permission is required for Dictate. Use the keyboard or grant it when you choose Dictate again."); return false }
        val token = generation
        return try {
            val next = factory.create(context)
            engine = next
            next.listen { text, final, error ->
                if (closed || generation != token || engine !== next) return@listen
                if (error != null) { stop(); status(error); return@listen }
                if (text != null) {
                    if (!instructionTextFits(text)) { stop(); status("Dictation exceeded the instruction text limit. The prior draft is preserved."); return@listen }
                    result(text, final)
                }
                if (final) { stop(); status("Dictation finished. Review the draft and choose Save.") }
            }
            if (engine === next) handler.postDelayed(deadline, 60_000)
            true
        } catch (_: Exception) { stop(); status("On-device dictation could not start. Check the device's speech model and microphone settings, or use the keyboard."); false }
    }
    fun stop() {
        generation++
        handler.removeCallbacks(deadline)
        val prior = engine; engine = null
        try { prior?.close() } catch (_: Exception) { if (!closed) status("The speech provider could not finish cleanup. Dictation remains stopped in this editor.") }
    }
    override fun close() { closed = true; stop() }
}
internal interface DictationEngine : AutoCloseable { fun listen(callback: (String?, Boolean, String?) -> Unit) }
internal interface DictationFactory {
    fun available(context: Context): Boolean
    fun create(context: Context): DictationEngine
}
internal object PlatformDictationFactory : DictationFactory {
    override fun available(context: Context): Boolean = Build.VERSION.SDK_INT >= 31 &&
        try { SpeechRecognizer.isOnDeviceRecognitionAvailable(context) } catch (_: Exception) { false }
    override fun create(context: Context): DictationEngine {
        if (Build.VERSION.SDK_INT < 31) throw UnsupportedOperationException("On-device dictation requires Android 12 or later")
        check(available(context))
        return createApi31(context)
    }
    @RequiresApi(31)
    private fun createApi31(context: Context): DictationEngine {
        val recognizer = SpeechRecognizer.createOnDeviceSpeechRecognizer(context)
        return object : DictationEngine {
            override fun listen(callback: (String?, Boolean, String?) -> Unit) {
                recognizer.setRecognitionListener(object : RecognitionListener {
                    override fun onReadyForSpeech(params: Bundle?) = Unit
                    override fun onBeginningOfSpeech() = Unit
                    override fun onRmsChanged(rmsdB: Float) = Unit
                    override fun onBufferReceived(buffer: ByteArray?) = Unit
                    override fun onEndOfSpeech() = Unit
                    override fun onEvent(eventType: Int, params: Bundle?) = Unit
                    override fun onError(error: Int) = callback(null, true, when (error) {
                        SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "Microphone permission was denied. Use the keyboard or review Android microphone settings."
                        SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED, SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE -> "The on-device speech model for this language is unavailable. Install it in the device's speech settings, or use the keyboard."
                        SpeechRecognizer.ERROR_NO_MATCH, SpeechRecognizer.ERROR_SPEECH_TIMEOUT -> "No text was recognized. The draft is unchanged; try again or use the keyboard."
                        else -> "The on-device recognizer stopped. Review the draft and retry explicitly, or use the keyboard."
                    })
                    private fun text(values: Bundle?): String? = values?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()
                    override fun onPartialResults(partialResults: Bundle?) = callback(text(partialResults), false, null)
                    override fun onResults(results: Bundle?) = callback(text(results), true, null)
                })
                recognizer.startListening(Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH)
                    .putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
                    .putExtra(RecognizerIntent.EXTRA_LANGUAGE, Locale.getDefault().toLanguageTag())
                    .putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, true)
                    .putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1))
            }
            override fun close() { try { recognizer.cancel() } finally { recognizer.destroy() } }
        }
    }
}
