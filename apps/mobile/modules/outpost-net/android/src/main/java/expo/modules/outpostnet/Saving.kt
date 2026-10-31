package expo.modules.outpostnet

import android.app.Activity
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore

/**
 * Copies of gallery items on the phone itself: pictures in Pictures/Outpost, videos in
 * Movies/Outpost, anything else (3D models) in Download/Outpost. Android 10+ lets an app add
 * its own files there without asking for storage access.
 */
object Saving {
  /** Copy a file from Outpost's cache to the phone's shared storage; returns its content:// URI. */
  fun save(ctx: Context, path: String, name: String, mime: String): Uri {
    if (Build.VERSION.SDK_INT < 29) throw IllegalStateException("Saving to the phone needs Android 10 or newer.")
    val src = java.io.File(path.removePrefix("file://"))
    if (!src.isFile) throw IllegalStateException("That file isn't on the phone yet.")
    val (collection, folder) = when {
      mime.startsWith("image/") -> MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY) to Environment.DIRECTORY_PICTURES
      mime.startsWith("video/") -> MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY) to Environment.DIRECTORY_MOVIES
      else -> MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY) to Environment.DIRECTORY_DOWNLOADS
    }
    val safe = name.replace(Regex("[^A-Za-z0-9._-]"), "_").take(80)
    val values = ContentValues().apply {
      put(MediaStore.MediaColumns.DISPLAY_NAME, safe)
      put(MediaStore.MediaColumns.MIME_TYPE, mime)
      put(MediaStore.MediaColumns.RELATIVE_PATH, "$folder/Outpost")
      put(MediaStore.MediaColumns.IS_PENDING, 1)
    }
    val resolver = ctx.contentResolver
    val uri = resolver.insert(collection, values) ?: throw IllegalStateException("The phone wouldn't make the file.")
    try {
      resolver.openOutputStream(uri)?.use { out -> src.inputStream().use { it.copyTo(out) } }
        ?: throw IllegalStateException("The phone wouldn't open the file.")
      resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
    } catch (e: Exception) {
      resolver.delete(uri, null, null)
      throw e
    }
    return uri
  }

  /** Android's share sheet for a saved file (to Bambu Handy, Drive, email...). */
  fun share(activity: Activity, uri: Uri, mime: String, title: String) {
    val send = Intent(Intent.ACTION_SEND).apply {
      type = mime
      putExtra(Intent.EXTRA_STREAM, uri)
      addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    activity.startActivity(Intent.createChooser(send, title).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
  }
}
