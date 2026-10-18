package com.visualworkbench.shared

import com.visualworkbench.bindings.core.CoreException
import com.visualworkbench.bindings.core.Point as NPoint
import com.visualworkbench.bindings.core.Transform as NTransform

public actual fun objectRotationTransforms(originals: List<Transform>, pivot: Point,
    first: Point, current: Point, snap15: Boolean): List<Transform> {
    if(originals.size !in 1..256)throw CoreFailure(CoreFailureKind.Invalid)
    prepareCoreLibrary()
    return try {
        com.visualworkbench.bindings.core.rotateObjectTransforms(
            originals.map{NTransform(it.a,it.b,it.c,it.d,it.e,it.f)},NPoint(pivot.x,pivot.y),
            NPoint(first.x,first.y),NPoint(current.x,current.y),snap15
        ).map{Transform(it.a,it.b,it.c,it.d,it.e,it.f)}
    }catch(error:CoreException){throw mapFailure(error)}
}
