package com.visualworkbench.desktop.documents

import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import com.visualworkbench.shared.*
import java.awt.image.BufferedImage

internal typealias PreparedDocumentTile = PreparedTile<ImageBitmap>
internal val PreparedDocumentTile.image: ImageBitmap get() = value

internal class DocumentTileCache(budgetBytes: Long = 192L * 1024 * 1024) : AutoCloseable {
    private val images = DocumentTileImages<ImageBitmap>(budgetBytes) { tile, current ->
        val source = BufferedImage(tile.width.toInt(), tile.height.toInt(), BufferedImage.TYPE_INT_ARGB)
        val row = IntArray(source.width)
        for (y in 0 until source.height) {
            if (!current()) throw DocumentFailure(DocumentFailureKind.Stale)
            for (x in row.indices) row[x] = documentArgb(tile.rgba, (y * source.width + x) * 4)
            source.setRGB(0, y, source.width, 1, row, 0, source.width)
        }
        val image = source.toComposeImageBitmap()
        image to { image.asSkiaBitmap().close() }
    }
    fun prepare(frame: DocumentDisplayFrame, stillCurrent: () -> Boolean): List<PreparedDocumentTile> = images.prepare(frame, stillCurrent)
    override fun close() = images.close()
}

internal fun DrawScope.drawDocumentTiles(tiles: List<PreparedDocumentTile>, current: DocumentFrameBinding) {
    if (tiles.any { it.binding != current }) throw DocumentFailure(DocumentFailureKind.Stale)
    for (tile in tiles) withTransform({
        translate(tile.source.origin.x.toFloat(), tile.source.origin.y.toFloat())
        scale((1.0 / tile.source.scale).toFloat(), (1.0 / tile.source.scale).toFloat(), androidx.compose.ui.geometry.Offset.Zero)
    }) { drawImage(tile.image, filterQuality = FilterQuality.Low) }
}
