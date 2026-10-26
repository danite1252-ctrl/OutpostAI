package expo.modules.outpostnet

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.util.Base64
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
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
  private var savedMode = AudioManager.MODE_NORMAL
  @Suppress("DEPRECATION") private var savedSpeaker = false

  /** Throws with a readable message when the microphone can't be opened. */
  fun start() {
    val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
    if (min <= 0) throw IllegalStateException("This phone's microphone can't record at 16 kHz.")
    // Call mode: echo cancelling knows what's playing and takes it back out of the microphone.
    savedMode = audio.mode
    try {
      audio.mode = AudioManager.MODE_IN_COMMUNICATION
      route()
    } catch (_: Exception) {
    }
    val rec = try {
      @Suppress("MissingPermission")
      AudioRecord(MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, maxOf(min, RATE))
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
    var floor = 300.0
    var inSpeech = false
    var loudRun = 0
    var spoken = 0
    var silent = 0
    var tick = 0
    while (running) {
      val n = rec.read(frame, 0, FRAME)
      if (n <= 0) continue
      var sum = 0.0
      for (i in 0 until n) sum += frame[i].toDouble() * frame[i]
      val rms = sqrt(sum / n)
      val playing = isPlaying()
      // The room's background level, learnt while nobody is talking.
      if (!inSpeech && !playing) floor = if (rms < floor) floor * 0.9 + rms * 0.1 else floor * 0.997 + rms * 0.003
      val base = maxOf(floor * 3.0, 450.0)
      // While a reply plays a little of it gets past echo cancelling: need clearer speech to interrupt.
      val threshold = if (playing) base * 2.2 else base
      val loud = rms > threshold
      if (++tick % 5 == 0) emit("onConverse", mapOf("type" to "level", "level" to (rms / (threshold * 3)).coerceIn(0.0, 1.0)))
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
        loudRun = if (loud) loudRun + 1 else 0
        if (loudRun >= (if (playing) BARGE_IN_FRAMES else START_FRAMES)) {
          inSpeech = true
          said.clear()
          said.addAll(preRoll) // Keep the start of the first word.
          preRoll.clear()
          spoken = loudRun
          silent = 0
          if (playing) interrupt()
          emit("onConverse", mapOf("type" to "start", "interrupted" to playing))
        }
      } else {
        said.add(copy)
        if (loud) {
          spoken++
          silent = 0
        } else {
          silent++
        }
        if (silent >= END_FRAMES || said.size >= MAX_FRAMES) {
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
    val data = ByteBuffer.allocate(samples * 2).order(ByteOrder.LITTLE_ENDIAN)
    for (f in frames) for (v in f) data.putShort(v)
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
    const val PRE_ROLL = 15 // 300 ms
    const val START_FRAMES = 4 // 80 ms of speech starts listening
    const val BARGE_IN_FRAMES = 12 // 240 ms to talk over a reply
    const val END_FRAMES = 40 // 800 ms of quiet ends it
    const val MIN_SPEECH_FRAMES = 15 // under 300 ms of speech is a cough or a bump
    const val MAX_FRAMES = 1500 // 30 s
  }
}
