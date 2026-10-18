package com.visualworkbench.android.documents

import android.graphics.*
import com.visualworkbench.shared.*

internal typealias PreparedDocumentTile = PreparedTile<Bitmap>
internal val PreparedDocumentTile.bitmap: Bitmap get() = value

internal class DocumentTileCache(budgetBytes: Long = 192L * 1024 * 1024) : AutoCloseable {
    private val images = DocumentTileImages<Bitmap>(budgetBytes) { tile, current ->
        val image = Bitmap.createBitmap(tile.width.toInt(), tile.height.toInt(), Bitmap.Config.ARGB_8888)
        try {
            val row = IntArray(image.width)
            for (y in 0 until image.height) {
                if (!current()) throw DocumentFailure(DocumentFailureKind.Stale)
                for (x in row.indices) row[x] = documentArgb(tile.rgba, (y * image.width + x) * 4)
                image.setPixels(row, 0, row.size, 0, y, row.size, 1)
            }
            image to { image.recycle() }
        } catch (error: Throwable) { image.recycle(); throw error } // Never published.
    }
    fun prepare(frame: DocumentDisplayFrame, stillCurrent: () -> Boolean): List<PreparedDocumentTile> = images.prepare(frame, stillCurrent)
    suspend fun prepareOwned(frame: DocumentDisplayFrame, stillCurrent: () -> Boolean): List<PreparedDocumentTile> = images.prepareOwned(frame, stillCurrent)
    override fun close() = images.close()
}

internal fun drawDocumentTiles(canvas: Canvas, tiles: List<PreparedDocumentTile>, current: DocumentFrameBinding) {
    if (tiles.any { it.binding != current }) throw DocumentFailure(DocumentFailureKind.Stale)
    val paint = Paint(Paint.FILTER_BITMAP_FLAG)
    for (tile in tiles) {
        canvas.save()
        try {
            canvas.translate(tile.source.origin.x.toFloat(), tile.source.origin.y.toFloat())
            canvas.scale((1.0 / tile.source.scale).toFloat(), (1.0 / tile.source.scale).toFloat())
            canvas.drawBitmap(tile.bitmap, 0f, 0f, paint)
        } finally { canvas.restore() }
    }
}
