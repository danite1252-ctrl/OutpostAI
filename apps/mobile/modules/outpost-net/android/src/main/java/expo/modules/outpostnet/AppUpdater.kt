package expo.modules.outpostnet

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * Updating the phone app from inside it: download the new APK (from the GitHub release),
 * then hand it to Android's package installer. Android only installs it if it's signed with
 * the same key as this app, so a tampered file can't get in.
 *
 * The first time, Android asks to allow Outpost to install apps and to confirm the update.
 * After that (Android 12+), Outpost installed itself, so updates go in without a prompt.
 */
class AppUpdater(
  private val context: () -> Context,
  private val emit: (String, Map<String, Any?>) -> Unit
) {
  private val client by lazy {
    // Ordinary HTTPS to GitHub (not the PC), so the system's certificate checks apply.
    OkHttpClient.Builder().connectTimeout(15, TimeUnit.SECONDS).readTimeout(2, TimeUnit.MINUTES).build()
  }
  private var receiver: BroadcastReceiver? = null

  private val action get() = "${context().packageName}.OUTPOST_UPDATE_STATUS"

  private fun dir(): File = File(context().cacheDir, "updates").apply { mkdirs() }

  /** What this phone allows: installing apps, and whether updates can skip the prompt. */
  fun status(): Map<String, Any?> {
    val ctx = context()
    val pm = ctx.packageManager
    val allowed = Build.VERSION.SDK_INT < 26 || pm.canRequestPackageInstalls()
    val installer = try {
      if (Build.VERSION.SDK_INT >= 30) pm.getInstallSourceInfo(ctx.packageName).installingPackageName
      else @Suppress("DEPRECATION") pm.getInstallerPackageName(ctx.packageName)
    } catch (_: Exception) {
      null
    }
    return mapOf(
      "canInstall" to allowed,
      // Updates need no tap when this app installed the current version itself (Android 12+).
      "silent" to (Build.VERSION.SDK_INT >= 31 && installer == ctx.packageName),
      "sdk" to Build.VERSION.SDK_INT
    )
  }

  /** Android's "Install unknown apps" page for Outpost. */
  fun openInstallSettings() {
    val ctx = context()
    val intent = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${ctx.packageName}"))
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    ctx.startActivity(intent)
  }

  /** A small text file over ordinary HTTPS (the release's latest.json on GitHub). */
  fun fetchText(url: String): String {
    require(url.startsWith("https://")) { "Only HTTPS." }
    client.newBuilder().callTimeout(20, TimeUnit.SECONDS).build()
      .newCall(Request.Builder().url(url).header("Accept", "application/json").build())
      .execute().use { resp ->
        if (!resp.isSuccessful) throw IllegalStateException("GitHub said no (${resp.code}).")
        val body = resp.body?.string() ?: ""
        if (body.length > 256 * 1024) throw IllegalStateException("That was too big to be release information.")
        return body
      }
  }

  /** The APK already downloaded under this name, if it's complete. */
  fun downloaded(name: String): String? {
    val f = File(dir(), safe(name))
    return if (f.isFile && f.length() > 0) f.absolutePath else null
  }

  /** Download in the background; resolves with the file's path. Old downloads are removed. */
  fun download(url: String, name: String, sha256: String?): String {
    require(url.startsWith("https://")) { "Updates only come over HTTPS." }
    val target = File(dir(), safe(name))
    dir().listFiles()?.filter { it.name != target.name }?.forEach { it.delete() }
    if (target.isFile && target.length() > 0) return target.absolutePath
    val part = File(dir(), "${target.name}.part")
    client.newCall(Request.Builder().url(url).build()).execute().use { resp ->
      if (!resp.isSuccessful) throw IllegalStateException("The download said no (${resp.code}).")
      val body = resp.body ?: throw IllegalStateException("The download was empty.")
      val total = body.contentLength()
      val digest = MessageDigest.getInstance("SHA-256")
      var got = 0L
      var lastPercent = -1
      part.outputStream().use { out ->
        body.byteStream().use { input ->
          val buf = ByteArray(64 * 1024)
          while (true) {
            val n = input.read(buf)
            if (n < 0) break
            out.write(buf, 0, n)
            digest.update(buf, 0, n)
            got += n
            val percent = if (total > 0) (got * 100 / total).toInt() else -1
            if (percent != lastPercent) {
              lastPercent = percent
              emit("onUpdate", mapOf("type" to "progress", "percent" to percent))
            }
          }
        }
      }
      val hex = digest.digest().joinToString("") { "%02x".format(it) }
      if (!sha256.isNullOrBlank() && !hex.equals(sha256, ignoreCase = true)) {
        part.delete()
        throw IllegalStateException("The download was damaged (its checksum didn't match). It will try again later.")
      }
    }
    if (!part.renameTo(target)) throw IllegalStateException("The download couldn't be saved.")
    return target.absolutePath
  }

  /**
   * Install a downloaded APK. Android closes Outpost while it updates. Progress and the result
   * arrive as onUpdate events: "confirm" (Android is showing its prompt), "failed".
   */
  fun install(path: String) {
    val ctx = context().applicationContext
    val file = File(path)
    require(file.isFile) { "The update file is missing." }
    val installer = ctx.packageManager.packageInstaller
    val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
      setAppPackageName(ctx.packageName)
      if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
    }
    listen(ctx)
    val sessionId = installer.createSession(params)
    installer.openSession(sessionId).use { session ->
      file.inputStream().use { input ->
        session.openWrite("outpost.apk", 0, file.length()).use { out ->
          input.copyTo(out)
          session.fsync(out)
        }
      }
      val intent = Intent(action).setPackage(ctx.packageName)
      val flags = PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0)
      val pending = PendingIntent.getBroadcast(ctx, sessionId, intent, flags)
      session.commit(pending.intentSender)
    }
  }

  private fun listen(ctx: Context) {
    if (receiver != null) return
    val r = object : BroadcastReceiver() {
      override fun onReceive(c: Context, intent: Intent) {
        when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
          PackageInstaller.STATUS_PENDING_USER_ACTION -> {
            @Suppress("DEPRECATION")
            val confirm = if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
            else intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
            if (confirm != null) {
              confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
              try {
                c.startActivity(confirm)
                emit("onUpdate", mapOf("type" to "confirm"))
              } catch (e: Exception) {
                emit("onUpdate", mapOf("type" to "failed", "message" to "Android didn't show its install prompt. Open Outpost and try again."))
              }
            }
          }
          PackageInstaller.STATUS_SUCCESS -> emit("onUpdate", mapOf("type" to "installed"))
          PackageInstaller.STATUS_FAILURE_ABORTED -> emit("onUpdate", mapOf("type" to "cancelled"))
          else -> {
            val why = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: "status $status"
            val message = if (status == PackageInstaller.STATUS_FAILURE_CONFLICT || status == PackageInstaller.STATUS_FAILURE_INCOMPATIBLE)
              "This update is signed differently from the app you have, so Android won't install it over it ($why)."
            else "The update didn't install ($why)."
            emit("onUpdate", mapOf("type" to "failed", "message" to message))
          }
        }
      }
    }
    val filter = IntentFilter(action)
    if (Build.VERSION.SDK_INT >= 33) ctx.registerReceiver(r, filter, Context.RECEIVER_NOT_EXPORTED)
    else @Suppress("UnspecifiedRegisterReceiverFlag") ctx.registerReceiver(r, filter)
    receiver = r
  }

  private fun safe(name: String) = name.replace(Regex("[^A-Za-z0-9._-]"), "_").take(80)
}
