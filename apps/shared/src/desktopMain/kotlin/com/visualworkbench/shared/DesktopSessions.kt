package com.visualworkbench.shared

import com.visualworkbench.bindings.core.openWindowsSessionService

/** Uses the fixed current-user DPAPI store; accepts no project storage path. */
public suspend fun createDesktopSessions(installationDeviceId: String): WorkbenchSessions {
    prepareCoreLibrary()
    return sessionFactory { openWindowsSessionService(installationDeviceId) }
}
