package com.visualworkbench.shared

import android.content.Context
import com.visualworkbench.shared.giant.AndroidGiantUriRuntime
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

/** The existing picker supplies its granted content URI. Native admission
 * precedes resolver open and bounds all staging. This function never spools
 * foreign bytes and does not take persistent URI permission or a device grant. */
public suspend fun openAndroidGiantOriginal(context: Context, options: GiantUriOptions): GiantPickedOriginal {
    currentCoroutineContext().ensureActive()
    prepareCoreLibrary()
    AndroidGiantUriRuntime.install(context)
    return openGiantContentUriNative(options)
}

/** Retry actual failed reader closes. A still-live reader keeps its original
 * native quota; directory cleanup failure remains conservatively retained. */
public suspend fun retryAndroidGiantUriRetirement(): Unit {
    currentCoroutineContext().ensureActive()
    prepareCoreLibrary()
    retryGiantUriRetirementNative()
}
