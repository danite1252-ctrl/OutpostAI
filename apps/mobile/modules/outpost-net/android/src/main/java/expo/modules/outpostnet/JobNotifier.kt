package expo.modules.outpostnet

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONObject
import java.security.KeyStore
import java.security.SecureRandom
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import javax.net.ssl.SSLContext

/**
 * "It's ready" notifications with the app closed: Android's WorkManager asks the paired
 * Outpost now and then (every 15 minutes, and every few minutes while something of yours is
 * still waiting or running) which jobs finished, and shows a notification for long ones (a 3D
 * model, a video, a training, or anything that took a minute or more).
 *
 * The connection (addresses, pinned certificate, sign-in) is kept in the app's own storage,
 * the sign-in encrypted with a key in the Android Keystore.
 */
object JobNotifier {
  private const val PREFS = "outpost-notify"
  private const val KEY_ALIAS = "outpost-notify-key"
  private const val CHANNEL = "jobs"
  private const val PERIODIC = "outpost-jobs-check"
  private const val SOON = "outpost-jobs-soon"

  private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  /** Keep checking for this connection (or stop, with `on` false). */
  fun configure(ctx: Context, on: Boolean, hosts: List<String>, port: Int, fingerprint: String, token: String) {
    val p = prefs(ctx).edit()
    if (!on || hosts.isEmpty() || token.isEmpty()) {
      p.clear().apply()
      WorkManager.getInstance(ctx).cancelUniqueWork(PERIODIC)
      WorkManager.getInstance(ctx).cancelUniqueWork(SOON)
      return
    }
    val old = prefs(ctx)
    val sameServer = old.getString("fingerprint", null) == fingerprint
    p.putString("hosts", hosts.joinToString(","))
      .putInt("port", port)
      .putString("fingerprint", fingerprint)
      .putString("token", seal(token))
    // A new pairing starts from now (no notifications for old jobs).
    if (!sameServer || old.getLong("since", 0) == 0L) p.putLong("since", System.currentTimeMillis() / 1000)
    p.apply()
    val periodic = PeriodicWorkRequestBuilder<JobCheckWorker>(15, TimeUnit.MINUTES)
      .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
      .build()
    WorkManager.getInstance(ctx).enqueueUniquePeriodicWork(PERIODIC, ExistingPeriodicWorkPolicy.UPDATE, periodic)
  }

  /** The app is open (true) or was left (false): no notifications while it's in front. */
  fun setInFront(ctx: Context, inFront: Boolean) {
    prefs(ctx).edit().putLong("front", if (inFront) System.currentTimeMillis() else 0L).apply()
    // Left the app: look again in a couple of minutes (and keep looking while jobs run).
    if (!inFront && prefs(ctx).contains("token")) soon(ctx, 2)
  }

  fun soon(ctx: Context, minutes: Long) {
    val once = OneTimeWorkRequestBuilder<JobCheckWorker>()
      .setInitialDelay(minutes, TimeUnit.MINUTES)
      .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
      .build()
    WorkManager.getInstance(ctx).enqueueUniqueWork(SOON, ExistingWorkPolicy.REPLACE, once)
  }

  /** One look (the WorkManager check); keeps looking soon while jobs run. */
  fun check(ctx: Context) {
    val left = look(ctx, 0) ?: return
    // Something still waiting or running: watch closely if Android allows it, and look
    // again soon either way.
    if (left > 0) {
      watch(ctx)
      soon(ctx, 3)
    }
  }

  /** Watch closely while jobs run (a foreground service holding the line open). Only works
   * while the app is open; it carries on after the app is left, until the jobs are done. */
  fun watch(ctx: Context) {
    if (!prefs(ctx).contains("token")) return
    try {
      androidx.core.content.ContextCompat.startForegroundService(ctx, android.content.Intent(ctx, JobWatchService::class.java))
    } catch (_: Exception) {
      // Android wouldn't start it (the app isn't in front): the regular checks cover it.
    }
  }

  /**
   * Ask the Outpost which of my jobs finished (waiting up to `wait` seconds for one), notify
   * about long ones, and return how many are still waiting or running; null if it can't be
   * reached.
   */
  fun look(ctx: Context, wait: Int): Int? {
    val p = prefs(ctx)
    val token = p.getString("token", null)?.let { open(it) } ?: return null
    val hosts = p.getString("hosts", "")!!.split(",").filter { it.isNotBlank() }
    val port = p.getInt("port", 0)
    val fingerprint = p.getString("fingerprint", "") ?: return null
    val since = p.getLong("since", System.currentTimeMillis() / 1000)
    val seen = p.getStringSet("seen", emptySet())!!.toMutableSet()
    val client = pinnedClient(fingerprint)
    var reply: JSONObject? = null
    for (host in hosts) {
      try {
        val req = Request.Builder()
          .url("https://$host:$port/api/jobs/finished?since=$since&wait=$wait")
          .header("Authorization", "Bearer $token")
          .header("Accept", "application/json")
          .build()
        client.newCall(req).execute().use { r ->
          if (r.code == 401) {
            // Unpaired: stop.
            configure(ctx, false, emptyList(), 0, "", "")
            return null
          }
          if (r.isSuccessful) reply = JSONObject(r.body?.string() ?: "{}")
        }
        if (reply != null) break
      } catch (_: Exception) {
        // Not at this address (away from home): try the next.
      }
    }
    val r = reply ?: return null
    val front = p.getLong("front", 0L)
    val inFront = front > 0 && System.currentTimeMillis() - front < 30 * 60 * 1000
    val jobs = r.optJSONArray("jobs")
    for (i in 0 until (jobs?.length() ?: 0)) {
      val j = jobs!!.getJSONObject(i)
      val id = j.optString("id")
      if (id.isEmpty() || !seen.add(id)) continue
      if (j.optBoolean("long") && !inFront) notify(ctx, id, j.optString("title"), j.optString("body"))
    }
    val now = r.optLong("now", System.currentTimeMillis() / 1000)
    p.edit()
      .putLong("since", maxOf(since, now - 5))
      .putStringSet("seen", if (seen.size > 100) seen.toList().takeLast(50).toSet() else seen)
      .apply()
    return r.optInt("unfinished", 0)
  }

  /** The small icon in the status bar. */
  internal val icon: Int get() = R.drawable.outpost_notify

  private fun notify(ctx: Context, id: String, title: String, body: String) {
    if (Build.VERSION.SDK_INT >= 33 &&
      ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) return
    val manager = ctx.getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Finished jobs", NotificationManager.IMPORTANCE_DEFAULT).apply {
          description = "When a 3D model, video, training or other long job is ready or fails"
        }
      )
    }
    val open = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)?.let {
      PendingIntent.getActivity(ctx, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
    val n = NotificationCompat.Builder(ctx, CHANNEL)
      .setSmallIcon(icon)
      .setContentTitle(title)
      .setContentText(body)
      .setStyle(NotificationCompat.BigTextStyle().bigText(body))
      .setAutoCancel(true)
      .apply { if (open != null) setContentIntent(open) }
      .build()
    try {
      NotificationManagerCompat.from(ctx).notify(id.hashCode(), n)
    } catch (_: SecurityException) {
      // Notifications were turned off for Outpost.
    }
  }

  private val clients = java.util.concurrent.ConcurrentHashMap<String, OkHttpClient>()

  private fun pinnedClient(fingerprint: String): OkHttpClient = clients.getOrPut(fingerprint) {
    val trust = PinnedTrust(fingerprint)
    val tls = SSLContext.getInstance("TLS")
    tls.init(null, arrayOf(trust), SecureRandom())
    return OkHttpClient.Builder()
      .sslSocketFactory(tls.socketFactory, trust)
      .hostnameVerifier { _, _ -> true }
      .connectTimeout(5, TimeUnit.SECONDS)
      // Long enough for the Outpost to wait for a job to finish (it answers within 50 s).
      .readTimeout(70, TimeUnit.SECONDS)
      .build()
  }

  // ------------------------------------------------------------ the sign-in, encrypted

  private fun key(): SecretKey {
    val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (ks.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
    val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    gen.init(
      KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .build()
    )
    return gen.generateKey()
  }

  private fun seal(text: String): String {
    val c = Cipher.getInstance("AES/GCM/NoPadding")
    c.init(Cipher.ENCRYPT_MODE, key())
    val out = c.iv + c.doFinal(text.toByteArray(Charsets.UTF_8))
    return Base64.encodeToString(out, Base64.NO_WRAP)
  }

  private fun open(sealed: String): String? = try {
    val raw = Base64.decode(sealed, Base64.NO_WRAP)
    val c = Cipher.getInstance("AES/GCM/NoPadding")
    c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, raw.copyOfRange(0, 12)))
    String(c.doFinal(raw.copyOfRange(12, raw.size)), Charsets.UTF_8)
  } catch (_: Exception) {
    null
  }
}

/** WorkManager's side: one look at the Outpost. */
class JobCheckWorker(ctx: Context, params: WorkerParameters) : Worker(ctx, params) {
  override fun doWork(): Result {
    JobNotifier.check(applicationContext)
    return Result.success()
  }
}
