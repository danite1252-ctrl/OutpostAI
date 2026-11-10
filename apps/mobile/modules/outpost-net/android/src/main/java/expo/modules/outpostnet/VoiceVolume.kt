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

  /**
   * A short two-note chime (rising: listening; falling: that didn't work), soft at the edges
   * so it's clear without being harsh. At the media volume, raised to the voice level if set.
   */
  fun chime(ctx: Context, up: Boolean) {
    try {
      val rate = 24000
      val notes = if (up) listOf(660.0 to 0.16, 990.0 to 0.26) else listOf(660.0 to 0.16, 440.0 to 0.26)
      val samples = ArrayList<Short>()
      for ((freq, secs) in notes) {
        val n = (rate * secs).toInt()
        for (i in 0 until n) {
          val t = i.toDouble() / rate
          // Quick fade in, gentle fade out (a bell-like tail), with a soft octave for warmth.
          val env = minOf(1.0, t / 0.012) * Math.exp(-3.5 * t / secs)
          val v = (Math.sin(2 * Math.PI * freq * t) + 0.25 * Math.sin(4 * Math.PI * freq * t)) / 1.25
          samples.add((v * env * 0.55 * Short.MAX_VALUE).toInt().toShort())
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
