package com.visualworkbench.shared.giant

import android.content.ContentResolver
import android.content.Context
import android.net.Uri
import com.visualworkbench.shared.GiantImageFailure
import com.visualworkbench.shared.GiantImageFailureKind
import com.visualworkbench.shared.GiantUriReaderOwner

/** Fixed root bridge. Creating a holder opens no source. Native first obtains
 * its strong holder reference and existing input admission, then opens/reads it.
 * No caller callback, FD, output path or retirement assertion is accepted. */
internal object AndroidGiantUriRuntime {
    @Volatile private var resolver: ContentResolver? = null
    @Volatile private var installed: Boolean = false

    @Synchronized internal fun install(context: Context): Unit {
        if (installed) return
        val application = context.applicationContext
            ?: throw GiantImageFailure(GiantImageFailureKind.Unsupported)
        resolver = application.contentResolver
        if (initializeNative() != 0) {
            resolver = null
            throw GiantImageFailure(GiantImageFailureKind.Unsupported)
        }
        installed = true
    }

    @JvmStatic @Synchronized private fun readerForNative(value: String): AndroidGiantUriReader {
        if (!installed || value.length !in 1..8192 || '\u0000' in value)
            throw IllegalArgumentException("URI intake unavailable")
        val uri = Uri.parse(value)
        if (uri.scheme != "content" || uri.authority.isNullOrEmpty())
            throw IllegalArgumentException("Unsupported input URI")
        return AndroidGiantUriReader(checkNotNull(resolver), uri)
    }

    @JvmStatic private external fun initializeNative(): Int
}

/** Only the fixed runtime constructs this holder; native pins it before open.
 * Failed close deliberately keeps the same reader. A successful JNI close call
 * is the sole native retirement witness; no client-supplied boolean exists. */
internal class AndroidGiantUriReader internal constructor(
    private val resolver: ContentResolver,
    private val uri: Uri,
) {
    private val owner = GiantUriReaderOwner {
        resolver.openInputStream(uri) ?: throw IllegalStateException("Input unavailable")
    }
    public fun openForNative(): Unit = owner.open()
    public fun readForNative(bytes: ByteArray): Int = owner.read(bytes)
    public fun closeForNative(): Unit = owner.close()
}
