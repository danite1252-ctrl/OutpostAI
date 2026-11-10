package expo.modules.outpostnet

import android.Manifest
import android.annotation.SuppressLint
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.os.BatteryManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Base64
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.sqrt

/**
 * The wake word: the phone listens (on the phone; nothing is sent) for "Hey Jarvis", even
 * with the screen off. When it hears it, it beeps, records what's said next, and sends that
 * to the Outpost, which has the chosen persona answer (its tools, your rules) and speaks the
 * answer back. A question that needs a yes or no ("Shall I unlock the front door?") is
 * listened for straight away. Then it goes back to listening for the word.
 *
 * A notification shows while it listens, with Stop. Other uses of the microphone in Outpost
 * (Talk, the Mic button) pause it.
 */
class WakeService : Service() {
  @Volatile private var running = false
  private var worker: Thread? = null
  private var watcher: Thread? = null
  private var lock: PowerManager.WakeLock? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      stop(this)
      return START_NOT_STICKY
    }
    try {
      show("Listening for \"${wordName(this)}\"")
    } catch (_: Exception) {
      // Android won't let the microphone start from the background (it restarted the service
      // on its own): it starts again when the app is next opened.
      stopSelf()
      return START_NOT_STICKY
    }
    alive = true
    if (worker?.isAlive != true) {
      running = true
      @SuppressLint("WakelockTimeout")
      val l = (getSystemService(POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "outpost:wake")
      l.acquire()
      lock = l
      worker = Thread({ listen() }, "outpost-wake").apply { start() }
    }
    if (watcher?.isAlive != true) {
      watcher = Thread({ watch() }, "outpost-wake-watch").apply { start() }
    }
    return START_STICKY
  }

  override fun onDestroy() {
    alive = false
    running = false
    worker?.interrupt()
    watcher?.interrupt()
    lock?.let { if (it.isHeld) it.release() }
    lock = null
    super.onDestroy()
  }

  /**
   * While it listens anyway, it also keeps a line open to the Outpost, so news (a routine's
   * result, the doorbell) arrives and is said the moment it's ready, not at Android's next check.
   */
  private fun watch() {
    try {
      while (running) {
        if (JobNotifier.look(applicationContext, 50, true) == null) Thread.sleep(20_000)
      }
    } catch (_: InterruptedException) {
    } catch (_: Exception) {
    }
  }

  // ------------------------------------------------------------------ listening

  private fun listen() {
    val p = prefs(this)
    val wordId = p.getString("word", "hey_jarvis") ?: "hey_jarvis"
    val threshold = p.getFloat("threshold", 0.5f)
    val detector = try {
      WakeWord.load(this, wordId)
    } catch (e: Exception) {
      fail("The wake word couldn't load: ${e.message}")
      return
    }
    detector.use {
      while (running) {
        if (held()) {
          Thread.sleep(300)
          continue
        }
        val rec = try {
          open()
        } catch (e: Exception) {
          fail(e.message ?: "The microphone didn't start.")
          return
        }
        try {
          val chunk = ShortArray(WakeWord.CHUNK)
          rec.startRecording()
          detector.reset()
          var floor = 300.0
          var hits = 0
          val recent = ArrayDeque<ShortArray>()
          while (running && !held()) {
            if (!readFully(rec, chunk)) break
            recent.addLast(chunk.copyOf())
            if (recent.size > RECENT_CHUNKS) recent.removeFirst()
            val level = rms(chunk)
            floor = if (level < floor) floor * 0.9 + level * 0.1 else floor * 0.995 + level * 0.005
            // It has to hold for two readings in a row (a single spike is TV or a bump).
            hits = if (detector.feed(chunk) >= threshold) hits + 1 else 0
            if (hits < PATIENCE) continue
            hits = 0
            // Then the Outpost checks it was really said, before it beeps.
            if (!confirmed(recent)) {
              p.edit().putInt("ignored", p.getInt("ignored", 0) + 1).apply()
              detector.reset()
              recent.clear()
              continue
            }
            recent.clear()
            // Heard it.
            p.edit().putInt("wakes", p.getInt("wakes", 0) + 1).putLong("last_wake", System.currentTimeMillis()).apply()
            conversation(rec, floor)
            detector.reset()
            show("Listening for \"${wordName(this)}\"")
          }
        } catch (_: InterruptedException) {
          return
        } catch (e: Exception) {
          p.edit().putString("error", e.message ?: "Listening stopped.").apply()
        } finally {
          try {
            rec.stop()
          } catch (_: Exception) {
          }
          rec.release()
        }
      }
    }
  }

  /**
   * After the word: what's said, the answer, and any yes or no it asks for. Music and other
   * sound on the phone (the car's stereo over Bluetooth) pause while it talks, so you can be
   * heard, and carry on afterwards.
   */
  private fun conversation(rec: AudioRecord, floor: Double) {
    val audio = getSystemService(AUDIO_SERVICE) as AudioManager
    val focus = if (Build.VERSION.SDK_INT >= 26) {
      android.media.AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT_EXCLUSIVE)
        .setAudioAttributes(
          android.media.AudioAttributes.Builder()
            .setUsage(android.media.AudioAttributes.USAGE_ASSISTANT)
            .setContentType(android.media.AudioAttributes.CONTENT_TYPE_SPEECH)
            .build()
        )
        .build()
    } else {
      null
    }
    try {
      if (Build.VERSION.SDK_INT >= 26 && focus != null) audio.requestAudioFocus(focus)
    } catch (_: Exception) {
    }
    // A moment for the music to fade out, so the chime isn't lost under it.
    try {
      Thread.sleep(300)
    } catch (_: InterruptedException) {
    }
    try {
      talk(rec, floor)
    } finally {
      try {
        if (Build.VERSION.SDK_INT >= 26 && focus != null) audio.abandonAudioFocusRequest(focus)
      } catch (_: Exception) {
      }
    }
  }

  private fun talk(rec: AudioRecord, floor: Double) {
    var brief = false
    var turns = 0
    while (running && turns++ < 4) {
      beep()
      show(if (brief) "Say yes or no" else "Listening…")
      val said = capture(rec, floor, brief, if (brief) 8000 else 5000) ?: return
      show("Thinking…")
      val answer = send(said) ?: return
      val audio = answer.optString("audio", "")
      val reply = answer.optString("reply", "")
      if (answer.optString("heard", "").isBlank() && reply.isBlank()) return // Nothing was said.
      if (audio.isNotEmpty()) {
        show(reply.take(80).ifBlank { "Answering…" })
        try {
          rec.stop()
        } catch (_: Exception) {
        }
        play(audio)
        rec.startRecording() // Fresh: nothing heard while it spoke.
      } else if (reply.isNotBlank()) {
        show(reply.take(120))
      }
      if (!answer.optBoolean("asking")) return
      brief = true
    }
  }

  /** What's said: from its start (within `startWithin` ms) to a pause; null if nothing. */
  private fun capture(rec: AudioRecord, floor: Double, brief: Boolean, startWithin: Long): ByteArray? {
    val frame = ShortArray(FRAME)
    val preRoll = ArrayDeque<ShortArray>()
    val said = ArrayList<ShortArray>()
    val threshold = maxOf(floor * 3, 460.0)
    var loudRun = 0
    var spoken = 0
    var silent = 0
    var inSpeech = false
    val deadline = System.currentTimeMillis() + startWithin
    while (running) {
      if (!readFully(rec, frame)) return null
      val loud = rms(frame) > threshold
      if (!inSpeech) {
        preRoll.addLast(frame.copyOf())
        if (preRoll.size > 15) preRoll.removeFirst()
        loudRun = if (loud) loudRun + 1 else 0
        if (loudRun >= 4) {
          inSpeech = true
          said.addAll(preRoll)
          spoken = loudRun
        } else if (System.currentTimeMillis() > deadline) {
          return null
        }
        continue
      }
      said.add(frame.copyOf())
      if (loud) {
        spoken++
        silent = 0
      } else silent++
      if (silent >= 40 || said.size >= 600) break // 0.8 s of quiet, or 12 s
    }
    if (spoken < (if (brief) 5 else 15)) return null
    val pcm = ByteArrayOutputStream()
    for (f in said) for (s in f) {
      pcm.write(s.toInt() and 0xff)
      pcm.write((s.toInt() shr 8) and 0xff)
    }
    return wav(pcm.toByteArray())
  }

  /** The last two seconds, checked by the Outpost (Whisper): was the word really said? */
  private fun confirmed(recent: ArrayDeque<ShortArray>): Boolean {
    val pcm = ByteArrayOutputStream()
    for (c in recent) for (x in c) {
      pcm.write(x.toInt() and 0xff)
      pcm.write((x.toInt() shr 8) and 0xff)
    }
    val p = prefs(this)
    val body = JSONObject()
      .put("audio", Base64.encodeToString(wav(pcm.toByteArray()), Base64.NO_WRAP))
      .put("word", wordName(this))
    val answer = JobNotifier.post(this, p, "/api/voice/heard", body, 10) ?: return false
    return answer.optBoolean("heard", false)
  }

  /** To the Outpost: what was said; back: what to say. */
  private fun send(audio: ByteArray): JSONObject? {
    val p = prefs(this)
    val body = JSONObject()
      .put("audio", Base64.encodeToString(audio, Base64.NO_WRAP))
      .put("persona_id", p.getString("persona", "") ?: "")
    val answer = JobNotifier.post(this, p, "/api/voice/turn", body, 150)
    if (answer == null) {
      show("Couldn't reach your Outpost")
      chime(false)
      Thread.sleep(1500)
      return null
    }
    answer.optString("error_message", "").takeIf { it.isNotBlank() }?.let {
      show(it.take(120))
      chime(false)
      Thread.sleep(1500)
      return null
    }
    return answer
  }

  private fun play(base64: String) {
    val file = File(cacheDir, "wake-reply.wav")
    try {
      file.writeBytes(Base64.decode(base64, Base64.NO_WRAP))
      VoiceVolume.loud(this) {
        val done = CountDownLatch(1)
        val player = MediaPlayer()
        player.setDataSource(file.absolutePath)
        player.setOnCompletionListener { done.countDown() }
        player.setOnErrorListener { _, _, _ -> done.countDown(); true }
        player.prepare()
        player.start()
        done.await(120, TimeUnit.SECONDS)
        player.release()
      }
    } catch (_: Exception) {
    } finally {
      file.delete()
    }
  }

  /** "I'm listening": a soft rising two-note chime. */
  private fun beep() = chime(true)

  private fun chime(up: Boolean) = VoiceVolume.chime(this, up)

  @SuppressLint("MissingPermission")
  private fun open(): AudioRecord {
    if (ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
      throw IllegalStateException("Outpost isn't allowed to use the microphone.")
    }
    val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
    val rec = AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(min, RATE))
    if (rec.state != AudioRecord.STATE_INITIALIZED) {
      rec.release()
      throw IllegalStateException("The microphone is busy.")
    }
    return rec
  }

  private fun readFully(rec: AudioRecord, buf: ShortArray): Boolean {
    var got = 0
    while (got < buf.size) {
      val n = rec.read(buf, got, buf.size - got)
      if (n <= 0) return false
      got += n
    }
    return true
  }

  private fun fail(why: String) {
    prefs(this).edit().putString("error", why).putBoolean("on", false).apply()
    running = false
    ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    stopSelf()
  }

  private fun show(text: String) {
    val manager = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Wake word", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Shown while Outpost listens for its wake word"
          setShowBadge(false)
        }
      )
    }
    val openApp = packageManager.getLaunchIntentForPackage(packageName)?.let {
      PendingIntent.getActivity(this, 3, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
    val stopIt = PendingIntent.getService(
      this, 4, Intent(this, WakeService::class.java).setAction(ACTION_STOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val n = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(JobNotifier.icon)
      .setContentTitle("Outpost wake word")
      .setContentText(text)
      .setOngoing(true)
      .setSilent(true)
      .setContentIntent(openApp)
      .addAction(0, "Stop", stopIt)
      .build()
    val type = if (Build.VERSION.SDK_INT >= 30) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
    ServiceCompat.startForeground(this, NOTIFICATION, n, type)
  }

  companion object {
    private const val PREFS = "outpost-wake"
    private const val CHANNEL = "wake"
    private const val NOTIFICATION = 7301
    private const val ACTION_STOP = "expo.modules.outpostnet.WAKE_STOP"
    private const val RATE = 16000
    private const val FRAME = 320 // 20 ms
    private const val PATIENCE = 2 // readings in a row (160 ms) before it counts
    private const val RECENT_CHUNKS = 25 // the last 2 s, for the check

    /** What else in Outpost is using the microphone (Talk, the Mic button): it pauses for them. */
    private val holders = java.util.Collections.synchronizedSet(HashSet<String>())

    fun hold(what: String, on: Boolean) {
      if (on) holders.add(what) else holders.remove(what)
    }

    private fun held() = holders.isNotEmpty()

    internal fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    private fun wordName(ctx: Context) =
      WakeWord.WORDS[prefs(ctx).getString("word", "hey_jarvis")]?.first ?: "Hey Jarvis"

    private fun battery(ctx: Context): Int =
      (ctx.getSystemService(Context.BATTERY_SERVICE) as BatteryManager).getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY)

    /** Start listening (the app must be open: Android only lets a microphone service start then). */
    fun start(ctx: Context, word: String, personaId: String, threshold: Double, hosts: List<String>, port: Int, fingerprint: String, token: String) {
      if (!WakeWord.WORDS.containsKey(word)) throw IllegalArgumentException("No wake word \"$word\".")
      if (personaId.isBlank()) throw IllegalArgumentException("Choose the persona that answers.")
      if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
        throw IllegalStateException("Allow Outpost to use the microphone first.")
      }
      JobNotifier.keepConnection(prefs(ctx), hosts, port, fingerprint, token)
      prefs(ctx).edit()
        .putBoolean("on", true)
        .putString("word", word)
        .putString("persona", personaId)
        .putFloat("threshold", threshold.toFloat().coerceIn(0.2f, 0.95f))
        .putLong("since", System.currentTimeMillis())
        .putInt("battery_start", battery(ctx))
        .putInt("wakes", 0)
        .putInt("ignored", 0)
        .remove("error")
        .apply()
      ContextCompat.startForegroundService(ctx, Intent(ctx, WakeService::class.java))
    }

    /** Listening now (this process). */
    @Volatile private var alive = false

    /**
     * When the app opens: if it was on (not switched off, nor stopped by a problem) and isn't
     * listening (the phone restarted), listen again, as it was set.
     */
    fun resume(ctx: Context, hosts: List<String>, port: Int, fingerprint: String, token: String) {
      val p = prefs(ctx)
      if (alive || !p.getBoolean("on", false) || p.getString("persona", "").isNullOrBlank()) return
      if (ContextCompat.checkSelfPermission(ctx, Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) return
      if (hosts.isNotEmpty() && token.isNotEmpty()) JobNotifier.keepConnection(p, hosts, port, fingerprint, token)
      p.edit()
        .putLong("since", System.currentTimeMillis())
        .putInt("battery_start", battery(ctx))
        .putInt("wakes", 0)
        .putInt("ignored", 0)
        .remove("error")
        .apply()
      ContextCompat.startForegroundService(ctx, Intent(ctx, WakeService::class.java))
    }

    fun stop(ctx: Context) {
      prefs(ctx).edit().putBoolean("on", false).apply()
      ctx.stopService(Intent(ctx, WakeService::class.java))
    }

    /** Whether it's listening, for what, since when, and what it has cost in battery. */
    fun status(ctx: Context): Map<String, Any?> {
      val p = prefs(ctx)
      val charging = (ctx.getSystemService(Context.BATTERY_SERVICE) as BatteryManager).isCharging
      return mapOf(
        "on" to p.getBoolean("on", false),
        "word" to p.getString("word", "hey_jarvis"),
        "persona" to p.getString("persona", null),
        "threshold" to p.getFloat("threshold", 0.5f).toDouble(),
        "since" to p.getLong("since", 0L).toDouble(),
        "batteryStart" to p.getInt("battery_start", -1),
        "batteryNow" to battery(ctx),
        "charging" to charging,
        "wakes" to p.getInt("wakes", 0),
        "ignored" to p.getInt("ignored", 0),
        "lastWake" to p.getLong("last_wake", 0L).toDouble(),
        "error" to p.getString("error", null),
        "words" to WakeWord.WORDS.map { (id, v) -> mapOf("id" to id, "name" to v.first) }
      )
    }

    /** The check: the detector on a recording of "Alexa" (should score high) and of other words (low). */
    fun check(ctx: Context): Map<String, Any?> {
      WakeWord.load(ctx, "alexa").use { d ->
        val yes = WakeWord.best(d, ctx.assets.open("wakeword/alexa_test.wav").use { it.readBytes() })
        val no = WakeWord.best(d, ctx.assets.open("wakeword/hey_jane.wav").use { it.readBytes() })
        return mapOf("heard" to yes.toDouble(), "other" to no.toDouble(), "ok" to (yes >= 0.5f && no < 0.5f))
      }
    }

    private fun rms(s: ShortArray): Double {
      var sum = 0.0
      for (x in s) sum += x.toDouble() * x
      return sqrt(sum / s.size)
    }

    private fun wav(pcm: ByteArray): ByteArray {
      val out = ByteArrayOutputStream()
      fun i32(v: Int) = out.write(byteArrayOf(v.toByte(), (v shr 8).toByte(), (v shr 16).toByte(), (v shr 24).toByte()))
      fun i16(v: Int) = out.write(byteArrayOf(v.toByte(), (v shr 8).toByte()))
      out.write("RIFF".toByteArray()); i32(36 + pcm.size); out.write("WAVEfmt ".toByteArray())
      i32(16); i16(1); i16(1); i32(RATE); i32(RATE * 2); i16(2); i16(16)
      out.write("data".toByteArray()); i32(pcm.size); out.write(pcm)
      return out.toByteArray()
    }
  }
}
