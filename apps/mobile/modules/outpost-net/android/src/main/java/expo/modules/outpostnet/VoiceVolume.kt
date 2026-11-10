package expo.modules.outpostnet

import android.content.Context
import android.media.AudioManager

/**
 * How loud Outpost speaks when nobody has the app open (the wake word's beep and replies, a
 * routine said on the phone). Those play at the phone's media volume, which is often low when
 * nothing else is playing; with a level set here, the media volume is raised to at least that
 * while Outpost speaks and put back afterwards.
 */
object VoiceVolume {
  private const val PREFS = "outpost-voice"
  private const val KEY = "level"

  /** 0: the phone's media volume as it is; otherwise at least this percent of the maximum. */
  fun level(ctx: Context): Int = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getInt(KEY, 0)

  fun setLevel(ctx: Context, percent: Int) {
    ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putInt(KEY, percent.coerceIn(0, 100)).apply()
  }

  private const val NEWS_KEY = "say_news"

  /** Routine news set to be said on the phone is said on this one (on unless turned off). */
  fun sayNews(ctx: Context): Boolean = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getBoolean(NEWS_KEY, true)

  fun setSayNews(ctx: Context, on: Boolean) {
    ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(NEWS_KEY, on).apply()
  }

  /**
   * The phone has been quietened: on silent or vibrate, in Do Not Disturb, or with the media
   * volume at nothing. Routine news then only notifies (and the volume is never raised for it).
   */
  fun quietened(ctx: Context): Boolean = try {
    val audio = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as android.app.NotificationManager
    audio.ringerMode != AudioManager.RINGER_MODE_NORMAL ||
      nm.currentInterruptionFilter != android.app.NotificationManager.INTERRUPTION_FILTER_ALL ||
      audio.getStreamVolume(AudioManager.STREAM_MUSIC) == 0
  } catch (_: Exception) {
    false
  }

  private val lock = Any()
  private var depth = 0
  private var saved = -1

  /** Run `block` with the media volume at least the chosen level, put back afterwards. */
  fun <T> loud(ctx: Context, block: () -> T): T {
    val audio = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    val pct = level(ctx)
    synchronized(lock) {
      if (depth++ == 0 && pct > 0) {
        try {
          val max = audio.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
          val now = audio.getStreamVolume(AudioManager.STREAM_MUSIC)
          val want = (max * pct + 99) / 100
          if (want > now) {
            saved = now
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, want, 0)
          }
        } catch (_: Exception) {
          // Do Not Disturb or a fixed-volume device: it plays as it is.
        }
      }
    }
    try {
      return block()
    } finally {
      synchronized(lock) {
        if (--depth == 0 && saved >= 0) {
          try {
            audio.setStreamVolume(AudioManager.STREAM_MUSIC, saved, 0)
          } catch (_: Exception) {
          }
          saved = -1
        }
      }
    }
  }

  /** Bluetooth audio (a car, headphones, hearing aids) is the way out: it needs a moment to wake up. */
  private fun bluetoothOut(audio: AudioManager): Boolean = try {
    audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).any {
      it.type == android.media.AudioDeviceInfo.TYPE_BLUETOOTH_A2DP ||
        it.type == android.media.AudioDeviceInfo.TYPE_HEARING_AID ||
        it.type == android.media.AudioDeviceInfo.TYPE_BLE_HEADSET ||
        it.type == android.media.AudioDeviceInfo.TYPE_BLE_SPEAKER
    }
  } catch (_: Exception) {
    false
  }

  /**
   * Three notes, about two thirds of a second: rising (C, E, G) for "listening", falling for
   * "that didn't work". Held at a steady level with soft edges, so it's obvious without being
   * harsh. A little silence first, more over Bluetooth, which otherwise swallows the start of
   * a sound while its link wakes up. At the media volume, raised to the voice level if set.
   */
  fun chime(ctx: Context, up: Boolean) {
    try {
      val audio = ctx.getSystemService(Context.AUDIO_SERVICE) as AudioManager
      val rate = 24000
      val notes = if (up) listOf(523.25 to 0.15, 659.25 to 0.15, 783.99 to 0.36) else listOf(659.25 to 0.15, 523.25 to 0.15, 392.0 to 0.36)
      val lead = if (bluetoothOut(audio)) 0.6 else 0.08
      val samples = ArrayList<Short>()
      repeat((rate * lead).toInt()) { samples.add(0) }
      for ((freq, secs) in notes) {
        val n = (rate * secs).toInt()
        for (i in 0 until n) {
          val t = i.toDouble() / rate
          // 10 ms in, 40 ms out, a slight settle in between; a soft octave and fifth for warmth.
          val edge = minOf(1.0, t / 0.010, (secs - t) / 0.040)
          val env = edge * (0.75 + 0.25 * Math.exp(-4.0 * t / secs))
          val v = (Math.sin(2 * Math.PI * freq * t) + 0.3 * Math.sin(4 * Math.PI * freq * t) + 0.12 * Math.sin(6 * Math.PI * freq * t)) / 1.42
          samples.add((v * env * 0.85 * Short.MAX_VALUE).toInt().toShort())
        }
      }
      val pcm = ShortArray(samples.size) { samples[it] }
      loud(ctx) {
        @Suppress("DEPRECATION")
        val track = android.media.AudioTrack(
          AudioManager.STREAM_MUSIC, rate, android.media.AudioFormat.CHANNEL_OUT_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT,
          pcm.size * 2, android.media.AudioTrack.MODE_STATIC
        )
        track.write(pcm, 0, pcm.size)
        track.play()
        Thread.sleep((pcm.size * 1000L / rate) + 80)
        track.release()
      }
    } catch (_: Exception) {
    }
  }
}
