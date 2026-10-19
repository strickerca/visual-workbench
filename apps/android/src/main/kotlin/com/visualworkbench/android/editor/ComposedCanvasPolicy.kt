package com.visualworkbench.android.editor

import com.visualworkbench.shared.DocumentSnapshot
import com.visualworkbench.shared.Shape

/** The native composed pixels include saved artwork and results, but not guides.
 * Keep these IDs in the controller's hit-test scene; only its render copy omits them. */
internal fun bakedCanonicalIds(document: DocumentSnapshot, composedPixelsCurrent: Boolean): Set<String> =
    if (!composedPixelsCurrent) emptySet() else document.render.items.asSequence()
        .filterNot { it.shape is Shape.Guide || it.shape is Shape.Adjustment }
        .map { it.objectId }.toSet()

/** Static baked IDs do not turn every frame into a transient front-buffer draw. */
internal fun transientHiddenIds(hidden: Set<String>, baked: Set<String>): Set<String> = hidden - baked
