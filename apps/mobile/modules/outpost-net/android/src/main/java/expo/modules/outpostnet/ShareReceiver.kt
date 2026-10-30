package expo.modules.outpostnet

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import android.util.Base64
import java.io.ByteArrayOutputStream
import java.io.File

/**
 * What another app shared with Outpost (Android's share sheet), or an Outpost link that was
 * opened (a pairing QR scanned with the camera).
 *
 * Shared files are copied into the app's cache (the sending app's permission to read them
 * ends soon), and pictures also come as a smaller JPEG data URI, ready to attach to a chat.
 */
object ShareReceiver {
  /** Largest file taken from a share. */
  private const val MAX_BYTES = 200L * 1024 * 1024
  /** Pictures for chat are scaled to fit this (pixels, longest side). */
  private const val MAX_SIDE = 2048

  fun dir(context: Context) = File(context.cacheDir, "shared")

  /** The share (or link) in `intent`, or null when it's an ordinary start. */
  fun read(context: Context, intent: Intent?): Map<String, Any?>? {
    intent ?: return null
    when (intent.action) {
      Intent.ACTION_VIEW -> {
        val link = intent.dataString ?: return null
        return mapOf("link" to link)
      }
      Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE -> {}
      else -> return null
    }
    val uris = mutableListOf<Uri>()
    if (intent.action == Intent.ACTION_SEND_MULTIPLE) {
      streamList(intent)?.let { uris.addAll(it) }
    } else {
      stream(intent)?.let { uris.add(it) }
    }
    val text = intent.getStringExtra(Intent.EXTRA_TEXT)?.takeIf { it.isNotBlank() }
    val subject = intent.getStringExtra(Intent.EXTRA_SUBJECT)?.takeIf { it.isNotBlank() }
    val out = dir(context).apply { mkdirs() }
    val files = uris.take(10).mapIndexedNotNull { i, uri -> copy(context, uri, out, i) }
    if (text == null && files.isEmpty()) return null
    return mapOf("text" to text, "subject" to subject, "files" to files)
  }

  @Suppress("DEPRECATION")
  private fun stream(intent: Intent): Uri? =
    if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
    else intent.getParcelableExtra(Intent.EXTRA_STREAM) as? Uri

  @Suppress("DEPRECATION")
  private fun streamList(intent: Intent): List<Uri>? =
    if (Build.VERSION.SDK_INT >= 33) intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
    else intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)

  private fun copy(context: Context, uri: Uri, out: File, index: Int): Map<String, Any?>? {
    return try {
      val resolver = context.contentResolver
      var name = "shared-$index"
      var size = -1L
      resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
        if (c.moveToFirst()) {
          c.getString(0)?.let { name = it }
          if (!c.isNull(1)) size = c.getLong(1)
        }
      }
      if (size > MAX_BYTES) return mapOf("name" to name, "error" to "too big")
      val mime = resolver.getType(uri) ?: guessMime(name)
      val safe = name.replace(Regex("[^A-Za-z0-9._ -]"), "_").take(100)
      val file = File(out, "${System.currentTimeMillis()}-$index-$safe")
      resolver.openInputStream(uri)?.use { input ->
        file.outputStream().use { o ->
          val buf = ByteArray(64 * 1024)
          var total = 0L
          while (true) {
            val n = input.read(buf)
            if (n < 0) break
            total += n
            if (total > MAX_BYTES) {
              o.close()
              file.delete()
              return mapOf("name" to name, "error" to "too big")
            }
            o.write(buf, 0, n)
          }
        }
      } ?: return null
      val picture = if (mime.startsWith("image/")) smallJpeg(file) else null
      mapOf(
        "path" to file.absolutePath,
        "name" to name,
        "mime" to mime,
        "size" to file.length().toDouble(),
        "dataUri" to picture,
      )
    } catch (e: Exception) {
      null
    }
  }

  private fun guessMime(name: String): String = when (name.substringAfterLast('.', "").lowercase()) {
    "pdf" -> "application/pdf"
    "docx" -> "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    "txt" -> "text/plain"
    "md" -> "text/markdown"
    "csv" -> "text/csv"
    "html", "htm" -> "text/html"
    "png" -> "image/png"
    "jpg", "jpeg" -> "image/jpeg"
    "webp" -> "image/webp"
    "mp4" -> "video/mp4"
    else -> "application/octet-stream"
  }

  /** A picture as a JPEG data URI no bigger than MAX_SIDE, for chat (null if unreadable). */
  private fun smallJpeg(file: File): String? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeFile(file.absolutePath, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MAX_SIDE) sample *= 2
    val bmp = BitmapFactory.decodeFile(file.absolutePath, BitmapFactory.Options().apply { inSampleSize = sample }) ?: return null
    val scale = MAX_SIDE.toFloat() / maxOf(bmp.width, bmp.height)
    val fitted = if (scale < 1f) Bitmap.createScaledBitmap(bmp, (bmp.width * scale).toInt(), (bmp.height * scale).toInt(), true) else bmp
    val bytes = ByteArrayOutputStream()
    fitted.compress(Bitmap.CompressFormat.JPEG, 88, bytes)
    if (fitted !== bmp) fitted.recycle()
    bmp.recycle()
    return "data:image/jpeg;base64," + Base64.encodeToString(bytes.toByteArray(), Base64.NO_WRAP)
  }

  /** Delete copies of shared files once they've been used. */
  fun clear(context: Context) {
    dir(context).deleteRecursively()
  }
}
