package expo.modules.outpostnet

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.util.Base64
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.math.abs
import kotlin.math.log10
import kotlin.math.sqrt

/**
 * Hands-free listening: the microphone stays open for the whole conversation, like a call.
 *
 * - The phone's echo cancelling (the same one calls use) removes the persona's own voice,
 *   so you can talk over a reply: speech while it plays stops it (`interrupt`).
 * - It notices when you start and stop talking itself, so there are no start/stop sounds.
 * - Each thing said arrives as an onConverse "utterance" event: a 16 kHz WAV, as base64,
 *   for the PC's Whisper.
 *
 * Audio is read on its own thread; [isPlaying] and [interrupt] come from [PhoneVoice].
 */
class Conversation(
  private val context: Context,
  private val bargeIn: Boolean,
  private val emit: (String, Map<String, Any?>) -> Unit,
  private val isPlaying: () -> Boolean,
  private val interrupt: () -> Unit
) {
  private val audio = context.getSystemService(Context.AUDIO_SERVICE) as AudioManager
  @Volatile private var running = false
  /** Paused: the microphone stays open but nothing is heard. */
  @Volatile var muted = false
  private var thread: Thread? = null
  private var record: AudioRecord? = null
  private var echo: AcousticEchoCanceler? = null
  private var noise: NoiseSuppressor? = null
  private var gain: AutomaticGainControl? = null
  private var savedMode = AudioManager.MODE_NORMAL
  @Suppress("DEPRECATION") private var savedSpeaker = false

  /** Throws with a readable message when the microphone can't be opened. */
  fun start() {
    val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
    if (min <= 0) throw IllegalStateException("This phone's microphone can't record at 16 kHz.")
    // The speech-recognition microphone (what Android's own recognizer uses): full level.
    // Call mode was tried first, but some phones (Samsung) record it near-silent at arm's length.
    // Echo cancelling is added as an effect where the phone has it; on top of that the reply's
    // own sound raises the bar for talking over it, and the PC checks the words aren't the reply.
    savedMode = audio.mode
    val rec = try {
      @Suppress("MissingPermission")
      AudioRecord(MediaRecorder.AudioSource.VOICE_RECOGNITION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(min, RATE))
    } catch (e: SecurityException) {
      restoreAudio()
      throw IllegalStateException("Outpost isn't allowed to use the microphone. Allow it in the phone's settings.")
    }
    if (rec.state != AudioRecord.STATE_INITIALIZED) {
      rec.release()
      restoreAudio()
      throw IllegalStateException("The microphone is in use by another app.")
    }
    if (AcousticEchoCanceler.isAvailable()) echo = AcousticEchoCanceler.create(rec.audioSessionId)?.apply { enabled = true }
    if (NoiseSuppressor.isAvailable()) noise = NoiseSuppressor.create(rec.audioSessionId)?.apply { enabled = true }
    // No automatic gain: it lifts the room noise in every pause, so a pause never sounds like one.
    // Quiet recordings are turned up in wav() instead.
    rec.startRecording()
    record = rec
    running = true
    thread = Thread({ listen(rec) }, "outpost-conversation").apply { start() }
    emit("onConverse", mapOf("type" to "ready", "echoCancelling" to (echo != null)))
  }

  fun stop() {
    running = false
    thread?.join(500)
    thread = null
    try {
      record?.stop()
    } catch (_: Exception) {
    }
    record?.release()
    record = null
    echo?.release()
    echo = null
    noise?.release()
    noise = null
    gain?.release()
    gain = null
    restoreAudio()
  }

  /** Replies to a headset if one is plugged in or paired, otherwise the loudspeaker (not the earpiece). */
  @Suppress("DEPRECATION")
  private fun route() {
    val headsets = setOf(
      AudioDeviceInfo.TYPE_WIRED_HEADSET,
      AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
      AudioDeviceInfo.TYPE_USB_HEADSET,
      AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
      26 // TYPE_BLE_HEADSET
    )
    if (Build.VERSION.SDK_INT >= 31) {
      val devices = audio.availableCommunicationDevices
      val pick = devices.firstOrNull { it.type in headsets } ?: devices.firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
      if (pick != null) audio.setCommunicationDevice(pick)
    } else {
      savedSpeaker = audio.isSpeakerphoneOn
      val outputs = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS)
      audio.isSpeakerphoneOn = outputs.none { it.type in headsets }
    }
  }

  @Suppress("DEPRECATION")
  private fun restoreAudio() {
    try {
      if (Build.VERSION.SDK_INT >= 31) audio.clearCommunicationDevice() else audio.isSpeakerphoneOn = savedSpeaker
      audio.mode = savedMode
    } catch (_: Exception) {
    }
  }

  private fun listen(rec: AudioRecord) {
    val frame = ShortArray(FRAME)
    val preRoll = ArrayDeque<ShortArray>()
    val said = ArrayList<ShortArray>()
    var floor = 60.0
    var inSpeech = false
    var loudRun = 0
    var spoken = 0
    var silent = 0
    var tick = 0
    /** Your loudest recent speech in this utterance, so a pause is judged against your voice. */
    var peak = 0.0
    /** The room's typical level (a TV, a fan), not just its quietest moments. */
    var noise = 60.0
    /** The last 0.8 s while talking: true for each quiet 20 ms. */
    val quietWindow = BooleanArray(END_FRAMES)
    var quietAt = 0
    /** How loud the reply sounds in the microphone while it plays. */
    var echoLevel = 0.0
    while (running) {
      val n = rec.read(frame, 0, FRAME)
      if (n <= 0) continue
      var sum = 0.0
      var crossings = 0
      for (i in 0 until n) {
        sum += frame[i].toDouble() * frame[i]
        if (i > 0 && (frame[i] >= 0) != (frame[i - 1] >= 0)) crossings++
      }
      val rms = sqrt(sum / n)
      // Voices cross zero at a moderate rate; hiss, clatter and fans much faster, hum much slower.
      val zcr = crossings.toDouble() / n
      val voiceLike = zcr in 0.01..0.32
      val playing = isPlaying()
      // The room's background level, learnt while nobody is talking.
      if (!inSpeech && !playing) {
        floor = if (rms < floor) floor * 0.9 + rms * 0.1 else floor * 0.997 + rms * 0.003
        noise = noise * 0.97 + rms * 0.03
      }
      // Judged against the room, not a fixed loudness: well above its quietest moments AND
      // its usual level, so steady background (a TV, a fan) doesn't count as you talking.
      val base = maxOf(floor * 3.0, noise * 2.2, 90.0)
      // While a reply plays a little of it gets past echo cancelling: need clearer speech to interrupt.
      if (playing && !inSpeech) echoLevel = echoLevel * 0.95 + rms * 0.05
      else if (!playing) echoLevel *= 0.98
      // Talking over a reply must be clearly louder than the reply sounds in the microphone.
      val threshold = if (playing) maxOf(base * 2.2, echoLevel * 3.0) else base
      val loud = rms > threshold && voiceLike
      if (++tick % 5 == 0) {
        // On a decibel scale above the room's background, so quiet and loud phones look alike.
        val above = 20 * log10(maxOf(rms, 1.0) / maxOf(floor, 1.0))
        emit(
          "onConverse",
          mapOf(
            "type" to "level",
            "level" to ((above - 4) / 26).coerceIn(0.0, 1.0),
            // For the check line on the hands-free screen.
            "db" to 20 * log10(maxOf(rms, 1.0) / 32768),
            "roomDb" to 20 * log10(maxOf(floor, 1.0) / 32768),
            "voiceLike" to voiceLike,
            "inSpeech" to inSpeech
          )
        )
      }
      if (muted || (playing && !bargeIn)) {
        inSpeech = false
        said.clear()
        preRoll.clear()
        loudRun = 0
        continue
      }
      val copy = frame.copyOf(n)
      if (!inSpeech) {
        preRoll.addLast(copy)
        while (preRoll.size > PRE_ROLL) preRoll.removeFirst()
        // Talking over a reply needs steady speech: most of the last ~0.4 s, not one bang.
        loudRun = if (loud) loudRun + 1 else if (playing) maxOf(0, loudRun - 2) else 0
        if (loudRun >= (if (playing) BARGE_IN_FRAMES else START_FRAMES)) {
          inSpeech = true
          said.clear()
          said.addAll(preRoll) // Keep the start of the first word.
          preRoll.clear()
          spoken = loudRun
          silent = 0
          peak = rms
          quietWindow.fill(false)
          quietAt = 0
          if (playing) interrupt()
          emit("onConverse", mapOf("type" to "start", "interrupted" to playing))
        }
      } else {
        said.add(copy)
        // Still talking: above the room AND within 12 dB of your own recent loudest.
        // Background noise sits well below your voice, so it counts as a pause even when
        // it's loud enough to be "above the room" (otherwise it would never end).
        val talkingNow = loud && rms > peak * 0.25
        if (talkingNow) {
          spoken++
          peak = maxOf(peak * 0.997, rms)
        } else {
          peak *= 0.997
        }
        quietWindow[quietAt] = !talkingNow
        quietAt = (quietAt + 1) % END_FRAMES
        // A pause: nearly all (85%) of the last 0.8 s was quiet, so a noise blip doesn't hold it open.
        val quiet = quietWindow.count { it }
        silent = if (said.size >= END_FRAMES) quiet else 0
        if (silent >= END_FRAMES * 85 / 100 || said.size >= MAX_FRAMES) {
          inSpeech = false
          loudRun = 0
          if (spoken >= MIN_SPEECH_FRAMES) {
            emit("onConverse", mapOf("type" to "utterance", "audio" to wav(said)))
          } else {
            emit("onConverse", mapOf("type" to "discard"))
          }
          said.clear()
        }
      }
    }
  }

  private fun wav(frames: List<ShortArray>): String {
    val samples = frames.sumOf { it.size }
    // Bring quiet recordings up to a good level for Whisper (peak at about 90%, at most 12x).
    var peak = 1
    for (f in frames) for (v in f) peak = maxOf(peak, abs(v.toInt()))
    val boost = (29500.0 / peak).coerceIn(1.0, 12.0)
    val data = ByteBuffer.allocate(samples * 2).order(ByteOrder.LITTLE_ENDIAN)
    for (f in frames) for (v in f) data.putShort((v * boost).toInt().coerceIn(-32768, 32767).toShort())
    val out = ByteArrayOutputStream(44 + samples * 2)
    val h = ByteBuffer.allocate(44).order(ByteOrder.LITTLE_ENDIAN)
    h.put("RIFF".toByteArray()).putInt(36 + samples * 2).put("WAVE".toByteArray())
    h.put("fmt ".toByteArray()).putInt(16).putShort(1).putShort(1).putInt(RATE).putInt(RATE * 2).putShort(2).putShort(16)
    h.put("data".toByteArray()).putInt(samples * 2)
    out.write(h.array())
    out.write(data.array())
    return Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)
  }

  companion object {
    const val RATE = 16000
    /** 20 ms. */
    const val FRAME = 320
    const val PRE_ROLL = 30 // 600 ms: keeps the first word, even after a pause for talking over
    const val START_FRAMES = 4 // 80 ms of speech starts listening
    const val BARGE_IN_FRAMES = 18 // ~360 ms of speech to pause a reply
    const val END_FRAMES = 40 // 800 ms of quiet ends it
    const val MIN_SPEECH_FRAMES = 15 // under 300 ms of speech is a cough or a bump
    const val MAX_FRAMES = 750 // 15 s
  }
}
