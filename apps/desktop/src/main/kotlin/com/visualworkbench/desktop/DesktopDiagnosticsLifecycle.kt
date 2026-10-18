package com.visualworkbench.desktop

/** EDT-owned caller policy. The exporter remains the only owner of its IO and
 * chooser; a refused close cannot advance the route or tear down its parents. */
internal class DesktopDiagnosticsLifecycle(
    private val held: () -> Boolean,
    private val requestClose: () -> Boolean,
    private val closeWithin: suspend () -> Boolean,
) {
    private var route: Any = Any()

    fun route(): Any = route

    fun allowsCommand(quitting: Boolean, diagnostics: Boolean = false, quit: Boolean = false): Boolean =
        !quitting && (!held() || diagnostics || quit)

    fun changeRoute(
        diagnostics: Boolean,
        quitting: Boolean,
        prepare: () -> Unit,
        apply: () -> Unit,
    ): Boolean {
        if (quitting || (held() && (diagnostics || !requestClose()))) return false
        if (diagnostics) prepare()
        route = Any()
        apply()
        return true
    }

    fun mayChoose(
        expectedRoute: Any,
        quitting: Boolean,
        displayable: Boolean,
        diagnostics: Boolean,
        competingModal: Boolean,
    ): Boolean = !quitting && displayable && diagnostics && !competingModal && route === expectedRoute && held()

    suspend fun closeParentsAfterRetirement(admit: () -> Boolean, closeParents: suspend () -> Unit): Boolean {
        if (!closeWithin()) return false
        // Drafts and other close predicates may have changed while IO settled.
        if (!admit()) return false
        closeParents()
        return true
    }
}
