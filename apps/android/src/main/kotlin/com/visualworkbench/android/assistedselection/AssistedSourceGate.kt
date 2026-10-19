package com.visualworkbench.android.assistedselection

import com.visualworkbench.shared.AssistedSelectionSource
import java.util.concurrent.atomic.AtomicReference

/** Worker-readable publication of one exact project reference and source. */
internal class AssistedSourceGate<P : Any> {
    private class Entry<P : Any>(val project: P, val source: AssistedSelectionSource)
    private val current = AtomicReference<Entry<P>?>(null)

    fun publish(project: P, source: AssistedSelectionSource) { current.set(Entry(project, source)) }
    fun sourceFor(project: P): AssistedSelectionSource? = current.get()?.takeIf { it.project === project }?.source
    fun matches(project: P, source: AssistedSelectionSource): Boolean =
        current.get()?.let { it.project === project && it.source == source } == true
    fun revoke() { current.set(null) }
}
