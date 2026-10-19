package com.visualworkbench.shared

/** Full-resolution paths retain their existing limits; this is display routing,
 * not permission to create/replace an original or reinterpret a PDF page. */
public fun giantDisplayRequired(document: DocumentSnapshot): Boolean =
    document.page == null && (document.width > 32_768u || document.height > 32_768u ||
        document.width.toULong() * document.height.toULong() > 50_000_000uL)

/** One physical view envelope including the precision panes. Base camera points
 * use canonical core mapping. Pane footprints are display-only affine inverses,
 * as in the pixel-grid overlay; they never alter input or document coordinates.
 * Planning at the highest pane scale preserves detail instead of stretching a
 * coarse base-view tile. An oversized union must refuse, never silently blur. */
public fun giantViewport(core: WorkbenchCore, camera: Camera, precision: PrecisionFrame = PrecisionFrame()): GiantViewport {
    require(camera.viewportWidth.isFinite() && camera.viewportHeight.isFinite() &&
        camera.viewportWidth > 0 && camera.viewportHeight > 0 && camera.scale.isFinite() && camera.scale > 0)
    val corners = core.mapPoints(camera, true, listOf(Point(0.0, 0.0), Point(camera.viewportWidth, 0.0),
        Point(camera.viewportWidth, camera.viewportHeight), Point(0.0, camera.viewportHeight))).toMutableList()
    require(corners.size == 4)
    var linear = core.cameraMatrix(camera); var scale = camera.scale
    for (pane in listOfNotNull(precision.lens, precision.loupe)) {
        val t = pane.matrix; val r = pane.rectangle
        require(listOf(t.a,t.b,t.c,t.d,t.e,t.f,r.x,r.y,r.width,r.height,pane.scale).all(Double::isFinite))
        require(r.width > 0 && r.height > 0 && pane.scale > 0)
        val det = t.a * t.d - t.b * t.c
        require(det.isFinite() && det != 0.0)
        for (p in listOf(Point(r.x,r.y),Point(r.x+r.width,r.y),Point(r.x+r.width,r.y+r.height),Point(r.x,r.y+r.height))) {
            val x = p.x-t.e; val y=p.y-t.f
            corners += Point((t.d*x-t.c*y)/det,(-t.b*x+t.a*y)/det)
        }
        if (pane.scale > scale) { scale=pane.scale;linear=t }
    }
    require(corners.all { it.x.isFinite() && it.y.isFinite() })
    return GiantViewport(corners.minOf{it.x},corners.minOf{it.y},corners.maxOf{it.x},corners.maxOf{it.y},
        linear.a,linear.b,linear.c,linear.d)
}

/** Repeated at actual draw, in addition to native admission and result adoption. */
public fun <T : Any> GiantVisibleFrame<T>.matches(binding: WorkflowBinding, epoch: Long, viewport: GiantViewport): Boolean =
    this.binding == binding && attachmentEpoch == epoch && view == viewport
