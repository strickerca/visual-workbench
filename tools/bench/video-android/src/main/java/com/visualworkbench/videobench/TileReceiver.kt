package com.visualworkbench.videobench

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.view.SurfaceHolder
import org.json.JSONObject
import java.io.ByteArrayInputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.InetSocketAddress
import java.net.Socket
import java.security.MessageDigest
import kotlin.math.ceil
import kotlin.math.min

/** Receives generated JPEG tiles through an owned adb TCP mapping, never an arbitrary host. */
internal object TileReceiver {
    fun receive(holder: SurfaceHolder, port: Int, active: () -> Boolean, progress: (Int, Int) -> Unit): JSONObject {
        require(port in 1024..65535)
        Socket().use { socket ->
            socket.soTimeout = 10000
            socket.tcpNoDelay = true
            socket.connect(InetSocketAddress("127.0.0.1", port), 10000)
            val input = DataInputStream(socket.getInputStream())
            val output = DataOutputStream(socket.getOutputStream())
            val magic = ByteArray(8).also { input.readFully(it) }
            require(magic.contentEquals("VWJS0001".toByteArray(Charsets.US_ASCII)))
            val width = input.readInt()
            val height = input.readInt()
            val count = input.readInt()
            val warmup = input.readInt()
            require(width == 2560 && height == 1440 && count in 3..250 && warmup in 1 until count)
            val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
            val canvas = Canvas(bitmap)
            canvas.drawColor(Color.BLACK)
            val paint = Paint(Paint.FILTER_BITMAP_FLAG)
            val durations = mutableListOf<Double>()
            var transferred = 0L
            try {
                for (i in 0 until count) {
                    check(active())
                    require(input.readInt() == i)
                    val length = input.readInt()
                    require(length in 4..8_388_608)
                    val bytes = ByteArray(length).also { input.readFully(it) }
                    transferred += length
                    val start = System.nanoTime()
                    val frame = DataInputStream(ByteArrayInputStream(bytes))
                    val tiles = frame.readInt()
                    require(tiles in 0..128)
                    for (tileIndex in 0 until tiles) {
                        val x = frame.readInt(); val y = frame.readInt()
                        val w = frame.readInt(); val h = frame.readInt()
                        val size = frame.readInt()
                        require(x >= 0 && y >= 0 && w in 1..width && h in 1..height && x.toLong() + w <= width && y.toLong() + h <= height)
                        require(size in 1..8_388_608 && size <= frame.available())
                        val encoded = ByteArray(size).also { frame.readFully(it) }
                        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
                        BitmapFactory.decodeByteArray(encoded, 0, size, bounds)
                        require(bounds.outWidth == w && bounds.outHeight == h && bounds.outMimeType == "image/jpeg")
                        val tile = requireNotNull(BitmapFactory.decodeByteArray(encoded, 0, size))
                        try { canvas.drawBitmap(tile, x.toFloat(), y.toFloat(), null) } finally { tile.recycle() }
                    }
                    require(frame.available() == 0)
                    val display = requireNotNull(holder.lockCanvas())
                    try {
                        display.drawColor(Color.BLACK)
                        val scale = min(display.width.toFloat() / width, display.height.toFloat() / height)
                        val dw = (width * scale).toInt(); val dh = (height * scale).toInt()
                        val left = (display.width - dw) / 2; val top = (display.height - dh) / 2
                        display.drawBitmap(bitmap, null, Rect(left, top, left + dw, top + dh), paint)
                    } finally { holder.unlockCanvasAndPost(display) }
                    if (i >= warmup) durations.add((System.nanoTime() - start) / 1e6)
                    output.writeInt(i)
                    output.write(MessageDigest.getInstance("SHA-256").digest(bytes))
                    output.flush()
                    if (i % 15 == 0) progress(i + 1, count)
                }
                val sorted = durations.sorted()
                require(sorted.isNotEmpty())
                fun at(p: Double) = sorted[ceil(p * sorted.size).toInt() - 1]
                return JSONObject().put("completed", true).put("posted_frames", count).put("measured_frames", count - warmup)
                    .put("width", width).put("height", height).put("received_payload_bytes", transferred)
                    .put("jpeg_decode_reconstruct_surface_post_ms", JSONObject().put("count", sorted.size).put("p50", at(.5)).put("p95", at(.95)).put("max", sorted.last()))
                    .put("scope", "Generated recorded dirty tiles; includes actual adb TCP delivery and phone JPEG decode/Surface posting, excludes concurrent PC capture/encode; posting is not photon presentation")
            } finally { bitmap.recycle() }
        }
    }
}
