package com.visualworkbench.shared

import android.app.Application

/** Application startup boundary, after the shared native loader loaded vw_core.
 * Does not read/write a credential or contact a provider. No Activity is retained. */
public object AndroidProviderRuntime {
    private var owner: Application? = null
    private var store: AndroidProviderKeyStore? = null
    private var initialized: Boolean = false

    @Synchronized
    public fun initialize(application: Application): AndroidProviderKeyStore {
        if (owner != null && owner !== application) unavailable()
        val selected = store ?: AndroidProviderKeyStore.production(application).also {
            owner = application
            store = it
        }
        if (!initialized) {
            val status = try { initializeNative(application, selected) } catch (_: Throwable) { 1 }
            if (status != 0) unavailable()
            initialized = true
        }
        return selected
    }
    @Synchronized
    public fun credentials(): AndroidProviderKeyStore {
        if (!initialized) unavailable()
        return store ?: unavailable()
    }
    private fun unavailable(): Nothing = throw ProviderCredentialException(ProviderCredentialProblem.Unavailable)
    @JvmStatic private external fun initializeNative(application: Application, store: AndroidProviderKeyStore): Int
}
