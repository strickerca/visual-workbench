package com.visualworkbench.desktop

/** Missing optional resources and failed integrity/lease admission have
 * different meanings. A refusal cannot silently configure a pen-only runtime. */
internal sealed interface DesktopRemoteCatalogAdmission {
    data object Absent:DesktopRemoteCatalogAdmission
    data class Ready(val catalog:DesktopRemoteCatalog):DesktopRemoteCatalogAdmission
    data object Refused:DesktopRemoteCatalogAdmission
}

internal fun loadRemoteCatalog(load:()->DesktopRemoteCatalog?):DesktopRemoteCatalogAdmission = try {
    load()?.let{DesktopRemoteCatalogAdmission.Ready(it)}?:DesktopRemoteCatalogAdmission.Absent
} catch(_:NativeRuntimeFailure) {
    DesktopRemoteCatalogAdmission.Refused
}

/** The ordinary Workbench remains usable. Remote capability creation and helper
 * configuration run only after this catalog admission boundary succeeds. */
internal fun <T> withAdmittedRemoteCatalog(admission:DesktopRemoteCatalogAdmission,block:(DesktopRemoteCatalog?)->T):T = when(admission) {
    DesktopRemoteCatalogAdmission.Absent->block(null)
    is DesktopRemoteCatalogAdmission.Ready->block(admission.catalog)
    DesktopRemoteCatalogAdmission.Refused->throw NativeRuntimeFailure("The packaged editor catalog could not be verified. Remote editing is unavailable; preserve the installed files and review the package.")
}
