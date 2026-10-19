package com.visualworkbench.shared

import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext

/** Guard exists before the first cancellable join. If that join is cancelled,
 * final retirement joins those same owners before restoring one observer set.
 * Callbacks retain the root's project/epoch and actual subscription ownership;
 * restore must refuse a closed/replaced project. Unknown retirement stops here. */
public suspend fun <T> withDocumentObserversRetired(
    retire: suspend () -> Unit,
    restore: suspend () -> Unit,
    adopt: suspend () -> T,
): T {
    try {
        retire()
        return adopt()
    } finally {
        withContext(NonCancellable) {
            retire()
            restore()
        }
    }
}
