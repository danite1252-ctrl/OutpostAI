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
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
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
      RoutineAlarm.cancel(ctx)
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
  fun look(ctx: Context, wait: Int, watch: Boolean = false): Int? {
    val p = prefs(ctx)
    val token = p.getString("token", null)?.let { open(it) } ?: return null
    val hosts = p.getString("hosts", "")!!.split(",").filter { it.isNotBlank() }
    val port = p.getInt("port", 0)
    val fingerprint = p.getString("fingerprint", "") ?: return null
    val since = p.getLong("since", System.currentTimeMillis() / 1000)
    val seen = p.getStringSet("seen", emptySet())!!.toMutableSet()
    val client = pinnedClient(fingerprint)
    var reply: JSONObject? = null
    var answered = ""
    for (host in hosts) {
      try {
        val req = Request.Builder()
          .url("https://$host:$port/api/jobs/finished?since=$since&wait=$wait" + (if (watch) "&watch=true" else ""))
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
        if (reply != null) {
          answered = host
          break
        }
      } catch (_: Exception) {
        // Not at this address (away from home): try the next.
      }
    }
    val r = reply ?: return null
    val front = p.getLong("front", 0L)
    val inFront = front > 0 && System.currentTimeMillis() - front < 30 * 60 * 1000
    val jobs = r.optJSONArray("jobs")
    val toSay = mutableListOf<Pair<String, String>>()
    for (i in 0 until (jobs?.length() ?: 0)) {
      val j = jobs!!.getJSONObject(i)
      val id = j.optString("id")
      if (id.isEmpty() || !seen.add(id)) continue
      if (j.optBoolean("long") && !inFront) notify(ctx, id, j.optString("title"), j.optString("body"), j.optString("chat_id"))
      val say = j.optString("say", "")
      if (say.isNotBlank()) toSay.add(say to j.optString("say_voice", ""))
    }
    // Routines set to speak on the phone: said out loud, one after another.
    for ((text, voice) in toSay) sayAloud(ctx, client, answered, port, token, text, voice)
    val now = r.optLong("now", System.currentTimeMillis() / 1000)
    // Routines: look again just after the next one's due, or soon if one's running now.
    val running = r.optInt("routines_running", 0)
    val next = r.optLong("next_routine_at", 0)
    when {
      running > 0 -> RoutineAlarm.set(ctx, System.currentTimeMillis() + 3 * 60 * 1000L)
      next > 0 -> RoutineAlarm.set(ctx, (next + 90) * 1000L - (now * 1000L - System.currentTimeMillis()))
      else -> RoutineAlarm.cancel(ctx)
    }
    p.edit()
      .putLong("since", maxOf(since, now - 5))
      .putStringSet("seen", if (seen.size > 100) seen.toList().takeLast(50).toSet() else seen)
      .apply()
    return r.optInt("unfinished", 0)
  }

  /**
   * Say a routine's result out loud (the server's voice, as the persona sounds). Waits until
   * it's finished; any failure is skipped (the notification still says it).
   */
  private fun sayAloud(ctx: Context, client: OkHttpClient, host: String, port: Int, token: String, text: String, voice: String) {
    if (host.isEmpty()) return
    var file: java.io.File? = null
    var player: android.media.MediaPlayer? = null
    try {
      val body = JSONObject().put("text", text).apply { if (voice.isNotEmpty()) put("voice", voice) }
        .toString().toRequestBody("application/json".toMediaType())
      val req = Request.Builder()
        .url("https://$host:$port/api/voice/speak")
        .header("Authorization", "Bearer $token")
        .post(body)
        .build()
      val bytes = client.newBuilder().readTimeout(120, TimeUnit.SECONDS).build().newCall(req).execute().use { r ->
        if (r.isSuccessful) r.body?.bytes() else null
      } ?: return
      val f = java.io.File(ctx.cacheDir, "said-${System.nanoTime()}.wav")
      f.writeBytes(bytes)
      file = f
      val done = java.util.concurrent.CountDownLatch(1)
      // The wake word doesn't listen to Outpost talking.
      WakeService.hold("said", true)
      val p = android.media.MediaPlayer()
      player = p
      p.setAudioAttributes(
        android.media.AudioAttributes.Builder()
          .setUsage(android.media.AudioAttributes.USAGE_MEDIA)
          .setContentType(android.media.AudioAttributes.CONTENT_TYPE_SPEECH)
          .build()
      )
      p.setDataSource(f.absolutePath)
      p.setOnCompletionListener { done.countDown() }
      p.setOnErrorListener { _, _, _ -> done.countDown(); true }
      p.prepare()
      VoiceVolume.loud(ctx) {
        p.start()
        done.await(3, TimeUnit.MINUTES)
      }
    } catch (_: Exception) {
      // Couldn't fetch or play it: the notification still says it.
    } finally {
      try { player?.release() } catch (_: Exception) {}
      file?.delete()
      WakeService.hold("said", false)
    }
  }

  /** Keep a connection (for the wake word) in its own storage, the sign-in encrypted. */
  fun keepConnection(p: android.content.SharedPreferences, hosts: List<String>, port: Int, fingerprint: String, token: String) {
    p.edit()
      .putString("hosts", hosts.joinToString(","))
      .putInt("port", port)
      .putString("fingerprint", fingerprint)
      .putString("token", seal(token))
      .apply()
  }

  /**
   * POST JSON to the Outpost kept in `p` (each of its addresses in turn). Its answer; a refusal
   * as `error_message`; null if it can't be reached.
   */
  fun post(ctx: Context, p: android.content.SharedPreferences, path: String, body: JSONObject, readSeconds: Long): JSONObject? {
    val token = p.getString("token", null)?.let { open(it) } ?: return null
    val hosts = p.getString("hosts", "")!!.split(",").filter { it.isNotBlank() }
    val port = p.getInt("port", 0)
    val fingerprint = p.getString("fingerprint", "") ?: return null
    val client = pinnedClient(fingerprint).newBuilder().readTimeout(readSeconds, TimeUnit.SECONDS).build()
    val payload = body.toString().toRequestBody("application/json".toMediaType())
    for (host in hosts) {
      try {
        val req = Request.Builder()
          .url("https://$host:$port$path")
          .header("Authorization", "Bearer $token")
          .post(payload)
          .build()
        client.newCall(req).execute().use { r ->
          val text = r.body?.string() ?: "{}"
          if (r.isSuccessful) return JSONObject(text)
          val message = try {
            JSONObject(text).optJSONObject("error")?.optString("message")
          } catch (_: Exception) {
            null
          }
          return JSONObject().put("error_message", message ?: if (r.code == 401) "Sign in to Outpost again." else "Outpost couldn't answer (${r.code}).")
        }
      } catch (_: Exception) {
        // Not at this address (away from home): try the next.
      }
    }
    return null
  }

  /** The small icon in the status bar. */
  internal val icon: Int get() = R.drawable.outpost_notify

  private fun notify(ctx: Context, id: String, title: String, body: String, chatId: String = "") {
    if (Build.VERSION.SDK_INT >= 33 &&
      ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) return
    val manager = ctx.getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Finished jobs", NotificationManager.IMPORTANCE_DEFAULT).apply {
          description = "When a 3D model, video, training or other long job is ready or fails, and routines' results"
        }
      )
    }
    val launch = ctx.packageManager.getLaunchIntentForPackage(ctx.packageName)
    // A routine's result: tapping opens its chat (outpost://chat/<id>, read by the app).
    val target = if (chatId.isNotEmpty() && launch?.component != null) {
      android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse("outpost://chat/$chatId"))
        .setComponent(launch.component)
        .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK or android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP)
    } else launch
    val open = target?.let {
      PendingIntent.getActivity(ctx, id.hashCode(), it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
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

/**
 * A routine's result shouldn't wait for Android's next 15-minute check (which an idle phone can
 * put off for hours): an alarm, which Android lets through while the phone is idle, looks just
 * after the next routine is due, and every few minutes while one is running.
 */
class RoutineAlarm : android.content.BroadcastReceiver() {
  override fun onReceive(ctx: Context, intent: android.content.Intent) {
    val pending = goAsync()
    Thread({
      try {
        JobNotifier.look(ctx.applicationContext, 0)
      } catch (_: Exception) {
        // Couldn't look now: the regular checks carry on.
      } finally {
        pending.finish()
      }
    }, "outpost-routine-alarm").start()
  }

  companion object {
    private fun intent(ctx: Context) = android.app.PendingIntent.getBroadcast(
      ctx,
      7,
      android.content.Intent(ctx, RoutineAlarm::class.java),
      android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_UPDATE_CURRENT,
    )

    /** Look at `atMillis` (wall clock). Inexact by a few minutes, but allowed while idle. */
    fun set(ctx: Context, atMillis: Long) {
      val alarms = ctx.getSystemService(android.app.AlarmManager::class.java) ?: return
      val at = maxOf(atMillis, System.currentTimeMillis() + 60_000)
      try {
        alarms.setAndAllowWhileIdle(android.app.AlarmManager.RTC_WAKEUP, at, intent(ctx))
      } catch (_: Exception) {
        // Not allowed here: the regular checks carry on.
      }
    }

    fun cancel(ctx: Context) {
      ctx.getSystemService(android.app.AlarmManager::class.java)?.cancel(intent(ctx))
    }
  }
}

