package com.visualworkbench.shared

import android.content.Context
import com.visualworkbench.bindings.core.openCallbackSessionService

/** Keystore-protected app identity. Pass the installation ID already used by
 * project creation; existing protected identities must match exactly. */
public suspend fun createAndroidSessions(context: Context, installationDeviceId: String): WorkbenchSessions {
    prepareCoreLibrary()
    return sessionFactory {
        val callback = try { AndroidTrustStore(context.applicationContext) } catch (_: Exception) { throw SessionFailure(SessionFailureKind.Storage) }
        openCallbackSessionService(installationDeviceId, callback)
    }
}
