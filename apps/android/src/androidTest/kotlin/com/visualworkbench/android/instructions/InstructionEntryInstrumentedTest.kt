package com.visualworkbench.android.instructions

import android.content.Context
import android.content.ContextWrapper
import android.content.pm.PackageManager
import androidx.test.platform.app.InstrumentationRegistry
import com.visualworkbench.shared.InstructionEntryMethod
import org.junit.Assert.*
import org.junit.Test

/** Every recognizer is injected. These cases request no permission, record no
 * audio and establish no installed OS speech/handwriting-provider acceptance. */
class InstructionEntryInstrumentedTest {
    @Test fun unavailableOnDeviceProviderDoesNotConstructAFallback() = main {
        val f = Fake(available = false); val adapter = OnDeviceDictation(context(true), { _, _ -> fail() }, {}, f)
        try { assertFalse(adapter.start()); assertEquals(0, f.created) } finally { adapter.close() }
    }
    @Test fun permissionDenialLeavesKeyboardPathAndNeverStartsMicrophone() = main {
        val f = Fake(); val errors = mutableListOf<String>(); val adapter = OnDeviceDictation(context(false), { _, _ -> fail() }, errors::add, f)
        try { assertFalse(adapter.start()); assertEquals(0, f.created); assertTrue(errors.single().contains("Microphone permission")) } finally { adapter.close() }
    }
    @Test fun lateStoppedGenerationCannotWriteTheNextRecognitionSession() = main {
        val f = Fake(); val outputs = mutableListOf<String>(); val adapter = OnDeviceDictation(context(true), { text, _ -> outputs.add(text) }, {}, f)
        try { assertTrue(adapter.start()); val first = f.engines.single(); first.callback("first partial", false, null)
            adapter.stop(); assertTrue(adapter.start()); first.callback("late first", true, null)
            f.engines.last().callback("second final", true, null)
            assertEquals(listOf("first partial", "second final"), outputs)
            assertEquals(1, first.closes); assertEquals(1, f.engines.last().closes)
        } finally { adapter.close() }
    }
    @Test fun closedAdapterRejectsLateCallbacksAndRetainsNoProvider() = main {
        val f = Fake(); val adapter = OnDeviceDictation(context(true), { _, _ -> fail("late callback") }, {}, f)
        assertTrue(adapter.start()); val engine = f.engines.single(); adapter.close()
        engine.callback("late", true, null); assertEquals(1, engine.closes); assertFalse(adapter.start())
    }
    @Test fun oversizedRecognitionResultIsRefusedWithoutTruncatingIntoTheDraft() = main {
        val f = Fake(); val outputs = mutableListOf<String>(); val adapter = OnDeviceDictation(context(true), { text, _ -> outputs.add(text) }, {}, f)
        try { adapter.start(); f.engines.single().callback("界".repeat(12000), true, null)
            assertTrue(outputs.isEmpty()); assertEquals(1, f.engines.single().closes)
        } finally { adapter.close() }
    }
    @Test fun nativeTextInputPreservesLiteralBoundsAndExplicitEntryMode() = main {
        val view = InstructionFieldView(context(false)); var callbacks = 0
        view.changed = { callbacks++; true }
        view.apply("before", InstructionEntryMethod.Handwriting, true)
        assertEquals(0, callbacks); assertEquals(InstructionEntryMethod.Handwriting, view.mode)
        view.append("界".repeat(12000)); assertEquals("before", view.text.toString())
        view.append(" after"); assertEquals("before after", view.text.toString())
        view.apply("model refresh", InstructionEntryMethod.PhoneKeyboard, false)
        assertFalse(view.isEnabled); assertEquals(InstructionEntryMethod.PhoneKeyboard, view.mode)
    }
    private class Engine : DictationEngine {
        lateinit var callback: (String?, Boolean, String?) -> Unit
        var closes = 0
        override fun listen(callback: (String?, Boolean, String?) -> Unit) { this.callback = callback }
        override fun close() { closes++ }
    }
    private class Fake(val available: Boolean = true) : DictationFactory {
        var created = 0; val engines = mutableListOf<Engine>()
        override fun available(context: Context) = available
        override fun create(context: Context): DictationEngine { created++; return Engine().also(engines::add) }
    }
    private fun context(allow: Boolean): Context = object : ContextWrapper(InstrumentationRegistry.getInstrumentation().targetContext) {
        override fun checkSelfPermission(permission: String) = if (allow) PackageManager.PERMISSION_GRANTED else PackageManager.PERMISSION_DENIED
    }
    private fun main(block: () -> Unit) { InstrumentationRegistry.getInstrumentation().runOnMainSync(block) }
}
