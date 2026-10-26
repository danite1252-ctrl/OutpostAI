package expo.modules.outpostnet

import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.MediaPlayer
import android.media.MediaRecorder
import android.media.audiofx.Visualizer
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.speech.RecognitionListener
import android.speech.RecognizerIntent
import android.speech.SpeechRecognizer
import android.util.Base64
import java.io.File

/**
 * The phone's side of voice:
 * - listening with Android's own speech recognizer (on the phone where it can be),
 * - recording a clip for the PC's Whisper when the phone has no recognizer,
 * - playing the PC's spoken replies.
 *
 * Android wants the recognizer and player used from the main thread, so everything here runs there.
 */
class PhoneVoice(
  private val context: () -> Context,
  private val emit: (String, Map<String, Any?>) -> Unit
) {
  private val main = Handler(Looper.getMainLooper())
  private var recognizer: SpeechRecognizer? = null
  /** Which recognizer to make next: the on-device one until it says it can't. */
  private var onDeviceWorks = true
  @Volatile private var player: MediaPlayer? = null
  /** A reply is audible right now (not paused, not between pieces). */
  @Volatile private var sounding = false
  /** Paused because someone may be talking over it; resumed if it was only noise. */
  private var held = false
  private var conversation: Conversation? = null
  /** Streams muted to silence the recognizer's start/stop beeps, to unmute afterwards. */
  private val beepsMuted = mutableListOf<Int>()
  private val unmute = Runnable { restoreBeeps() }
  private var playing: File? = null
  /** Measures the reply as it plays, for the hands-free wave. */
  private var visualizer: Visualizer? = null
  private val meter = object : Runnable {
    override fun run() {
      val v = visualizer ?: return
      try {
        val m = Visualizer.MeasurementPeakRms()
        if (v.getMeasurementPeakRms(m) == Visualizer.SUCCESS) {
          // mRms is in millibels: speech sits around -35 to -10 dB.
          val level = ((m.mRms / 100.0) + 45.0) / 35.0
          emit("onPlayback", mapOf("type" to "level", "level" to level.coerceIn(0.0, 1.0)))
        }
      } catch (_: Exception) {
      }
      main.postDelayed(this, 50)
    }
  }
  private var recorder: MediaRecorder? = null
  private var recording: File? = null
  private var watching: Runnable? = null

  fun support(): Map<String, Any?> {
    val ctx = context()
    val any = SpeechRecognizer.isRecognitionAvailable(ctx)
    val onDevice = Build.VERSION.SDK_INT >= 31 && SpeechRecognizer.isOnDeviceRecognitionAvailable(ctx)
    return mapOf("recognizer" to (any || onDevice), "onDevice" to onDevice)
  }

  // ---------------------------------------------------------------- listening

  fun listen(language: String?, partial: Boolean) = main.post { startListening(language, partial, true) }

  private fun startListening(language: String?, partial: Boolean, retry: Boolean) {
    destroyRecognizer()
    muteBeeps()
    val ctx = context()
    val onDevice = onDeviceWorks && Build.VERSION.SDK_INT >= 31 && SpeechRecognizer.isOnDeviceRecognitionAvailable(ctx)
    val r = if (onDevice) SpeechRecognizer.createOnDeviceSpeechRecognizer(ctx) else SpeechRecognizer.createSpeechRecognizer(ctx)
    recognizer = r
    r.setRecognitionListener(object : RecognitionListener {
      override fun onReadyForSpeech(params: Bundle?) = emit("onSpeech", mapOf("type" to "ready"))
      override fun onBeginningOfSpeech() = emit("onSpeech", mapOf("type" to "speaking"))
      override fun onRmsChanged(rmsdB: Float) {
        // Roughly -2 (silence) to 10 (loud).
        emit("onSpeech", mapOf("type" to "level", "level" to ((rmsdB + 2f) / 12f).coerceIn(0f, 1f).toDouble()))
      }
      override fun onBufferReceived(buffer: ByteArray?) {}
      override fun onEndOfSpeech() {}
      override fun onEvent(eventType: Int, params: Bundle?) {}

      override fun onPartialResults(results: Bundle?) {
        val text = best(results) ?: return
        emit("onSpeech", mapOf("type" to "partial", "text" to text))
      }

      override fun onResults(results: Bundle?) {
        emit("onSpeech", mapOf("type" to "final", "text" to (best(results) ?: "")))
        main.post { destroyRecognizer() }
        laterRestoreBeeps()
      }

      override fun onError(error: Int) {
        val languageMissing = error == SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED ||
          error == SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE
        if (onDevice && retry && (languageMissing || error == SpeechRecognizer.ERROR_SERVER || error == SpeechRecognizer.ERROR_CLIENT)) {
          // The on-device recognizer can't do this language (yet): use the phone's normal one.
          onDeviceWorks = false
          main.post { startListening(language, partial, false) }
          return
        }
        main.post { destroyRecognizer() }
        laterRestoreBeeps()
        val quiet = error == SpeechRecognizer.ERROR_NO_MATCH || error == SpeechRecognizer.ERROR_SPEECH_TIMEOUT
        emit(
          "onSpeech",
          if (quiet) mapOf("type" to "final", "text" to "")
          else mapOf("type" to "error", "code" to error, "message" to describe(error))
        )
      }
    })
    val intent = Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).apply {
      putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
      putExtra(RecognizerIntent.EXTRA_PARTIAL_RESULTS, partial)
      putExtra(RecognizerIntent.EXTRA_PREFER_OFFLINE, true)
      putExtra(RecognizerIntent.EXTRA_MAX_RESULTS, 1)
      putExtra(RecognizerIntent.EXTRA_SPEECH_INPUT_COMPLETE_SILENCE_LENGTH_MILLIS, 1200L)
      putExtra(RecognizerIntent.EXTRA_SPEECH_INPUT_POSSIBLY_COMPLETE_SILENCE_LENGTH_MILLIS, 1200L)
      if (!language.isNullOrBlank()) putExtra(RecognizerIntent.EXTRA_LANGUAGE, language)
    }
    try {
      r.startListening(intent)
    } catch (e: Exception) {
      destroyRecognizer()
      emit("onSpeech", mapOf("type" to "error", "code" to -1, "message" to (e.message ?: "The phone's speech recognizer didn't start.")))
    }
  }

  /** Finish now and give what was heard. */
  fun stopListening() = main.post { recognizer?.stopListening() }

  /** Stop without a result. */
  fun cancelListening() = main.post {
    recognizer?.cancel()
    destroyRecognizer()
    laterRestoreBeeps()
  }

  /**
   * Android's recognizer plays a sound when it starts and stops. It can't be turned off,
   * so the sound streams are muted while it listens (and only those Outpost muted are unmuted).
   */
  private fun muteBeeps() {
    main.removeCallbacks(unmute)
    val audio = context().getSystemService(Context.AUDIO_SERVICE) as AudioManager
    for (stream in listOf(AudioManager.STREAM_MUSIC, AudioManager.STREAM_NOTIFICATION, AudioManager.STREAM_SYSTEM)) {
      if (stream in beepsMuted) continue
      try {
        if (!audio.isStreamMute(stream)) {
          audio.adjustStreamVolume(stream, AudioManager.ADJUST_MUTE, 0)
          beepsMuted.add(stream)
        }
      } catch (_: Exception) {
        // Do Not Disturb can refuse this; then the beep plays.
      }
    }
  }

  /** The stop sound plays just after the result, so wait a moment before unmuting. */
  private fun laterRestoreBeeps() = main.post {
    main.removeCallbacks(unmute)
    main.postDelayed(unmute, 700)
  }

  private fun restoreBeeps() {
    if (beepsMuted.isEmpty()) return
    val audio = context().getSystemService(Context.AUDIO_SERVICE) as AudioManager
    for (stream in beepsMuted) {
      try {
        audio.adjustStreamVolume(stream, AudioManager.ADJUST_UNMUTE, 0)
      } catch (_: Exception) {
      }
    }
    beepsMuted.clear()
  }

  // ---------------------------------------------------------------- hands-free (see Conversation)

  fun converse(bargeIn: Boolean) = main.post {
    stopConversationNow()
    val c = Conversation(
      context(),
      bargeIn,
      emit,
      isPlaying = { sounding },
      // Only pause: if it turns out to be noise, the reply carries on from where it was.
      interrupt = { main.post { holdNow() } }
    )
    try {
      c.start()
      conversation = c
    } catch (e: Exception) {
      emit("onConverse", mapOf("type" to "error", "message" to (e.message ?: "The microphone didn't start.")))
    }
  }

  fun converseMuted(muted: Boolean) = main.post { conversation?.muted = muted }

  fun stopConversation() = main.post { stopConversationNow() }

  private fun stopConversationNow() {
    val c = conversation ?: return
    conversation = null
    c.stop()
  }

  private fun destroyRecognizer() {
    val r = recognizer ?: return
    recognizer = null
    try {
      r.destroy()
    } catch (_: Exception) {
    }
  }

  private fun best(results: Bundle?): String? =
    results?.getStringArrayList(SpeechRecognizer.RESULTS_RECOGNITION)?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }

  private fun describe(error: Int): String = when (error) {
    SpeechRecognizer.ERROR_AUDIO -> "The microphone didn't work."
    SpeechRecognizer.ERROR_INSUFFICIENT_PERMISSIONS -> "Outpost isn't allowed to use the microphone. Allow it in the phone's settings."
    SpeechRecognizer.ERROR_NETWORK, SpeechRecognizer.ERROR_NETWORK_TIMEOUT ->
      "The phone's speech recognizer needs the internet for this language. Download the language for offline use in the phone's settings."
    SpeechRecognizer.ERROR_RECOGNIZER_BUSY -> "The phone's speech recognizer is busy. Try again."
    SpeechRecognizer.ERROR_LANGUAGE_NOT_SUPPORTED, SpeechRecognizer.ERROR_LANGUAGE_UNAVAILABLE ->
      "The phone can't recognise this language. Download it in the phone's speech settings."
    else -> "The phone's speech recognizer stopped (error $error)."
  }

  // ---------------------------------------------------------------- recording (for Whisper on the PC)

  /**
   * Record a clip. With `autoStop`, it ends by itself after a pause in speech (hands-free);
   * otherwise when [stopRecording] is called. Either way it arrives as an onRecording "done"
   * event carrying the clip as base64 (M4A), or empty if nothing was said.
   */
  fun record(autoStop: Boolean) = main.post {
    cancelRecordingNow()
    val ctx = context()
    val file = File(File(ctx.cacheDir, "outpost-media").apply { mkdirs() }, "mic-${System.currentTimeMillis()}.m4a")
    @Suppress("DEPRECATION")
    val rec = if (Build.VERSION.SDK_INT >= 31) MediaRecorder(ctx) else MediaRecorder()
    try {
      rec.setAudioSource(MediaRecorder.AudioSource.VOICE_RECOGNITION)
      rec.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
      rec.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
      rec.setAudioChannels(1)
      rec.setAudioSamplingRate(16000)
      rec.setAudioEncodingBitRate(32000)
      rec.setOutputFile(file.absolutePath)
      rec.prepare()
      rec.start()
    } catch (e: Exception) {
      try {
        rec.release()
      } catch (_: Exception) {
      }
      file.delete()
      emit("onRecording", mapOf("type" to "error", "message" to (e.message ?: "The microphone didn't start.")))
      return@post
    }
    recorder = rec
    recording = file
    rec.maxAmplitude // The first reading is always 0; start the count.
    val started = System.currentTimeMillis()
    var floor = 0
    var spoke = 0L
    var quietSince = 0L
    val tick = object : Runnable {
      override fun run() {
        if (recorder !== rec) return
        val amp = try {
          rec.maxAmplitude
        } catch (_: Exception) {
          0
        }
        val now = System.currentTimeMillis()
        // Learn the room's background level in the first half second.
        if (now - started < 500) floor = maxOf(floor, amp)
        val threshold = maxOf(1500, floor * 5 / 2)
        emit("onRecording", mapOf("type" to "level", "level" to (amp.toDouble() / (threshold * 4)).coerceIn(0.0, 1.0)))
        if (now - started >= 500) {
          if (amp > threshold) {
            if (spoke == 0L) spoke = now
            quietSince = 0L
          } else if (spoke != 0L && quietSince == 0L) {
            quietSince = now
          }
        }
        if (autoStop) {
          val paused = spoke != 0L && quietSince != 0L && now - quietSince > 1200 && quietSince - spoke > 300
          val long = spoke != 0L && now - spoke > 30000
          val nothing = spoke == 0L && now - started > 20000
          if (paused || long) {
            finishRecording(true)
            return
          }
          if (nothing) {
            finishRecording(false)
            return
          }
        }
        main.postDelayed(this, 80)
      }
    }
    watching = tick
    main.postDelayed(tick, 80)
  }

  fun stopRecording() = main.post { finishRecording(true) }

  fun cancelRecording() = main.post { cancelRecordingNow() }

  private fun finishRecording(keep: Boolean) {
    val rec = recorder ?: return
    val file = recording
    recorder = null
    recording = null
    watching?.let { main.removeCallbacks(it) }
    watching = null
    var ok = true
    try {
      rec.stop()
    } catch (_: Exception) {
      ok = false // Stopped before any sound was captured.
    }
    try {
      rec.release()
    } catch (_: Exception) {
    }
    val audio = if (keep && ok && file != null && file.length() > 0) Base64.encodeToString(file.readBytes(), Base64.NO_WRAP) else ""
    file?.delete()
    emit("onRecording", mapOf("type" to "done", "audio" to audio))
  }

  private fun cancelRecordingNow() {
    val rec = recorder ?: return
    recorder = null
    watching?.let { main.removeCallbacks(it) }
    watching = null
    try {
      rec.stop()
    } catch (_: Exception) {
    }
    try {
      rec.release()
    } catch (_: Exception) {
    }
    recording?.delete()
    recording = null
  }

  // ---------------------------------------------------------------- playing

  /** Play a downloaded clip (file://...), deleting it afterwards; onPlayback "end" when done. */
  fun play(uri: String) = main.post {
    stopPlayingNow(false)
    val file = File(uri.removePrefix("file://"))
    val p = MediaPlayer()
    player = p
    playing = file
    p.setAudioAttributes(
      AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_ASSISTANT)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        .build()
    )
    p.setOnCompletionListener { finished(p, null) }
    p.setOnErrorListener { _, what, _ ->
      finished(p, "The reply couldn't be played ($what).")
      true
    }
    try {
      p.setDataSource(file.absolutePath)
      p.setOnPreparedListener {
        it.start()
        sounding = true
        held = false
        startMeter(it)
      }
      p.prepareAsync()
    } catch (e: Exception) {
      finished(p, e.message ?: "The reply couldn't be played.")
    }
  }

  private fun startMeter(p: MediaPlayer) {
    stopMeter()
    try {
      visualizer = Visualizer(p.audioSessionId).apply {
        measurementMode = Visualizer.MEASUREMENT_MODE_PEAK_RMS
        enabled = true
      }
      main.post(meter)
    } catch (_: Exception) {
      // Needs the microphone permission on some phones; the wave animates without it.
      visualizer = null
    }
  }

  private fun stopMeter() {
    main.removeCallbacks(meter)
    try {
      visualizer?.release()
    } catch (_: Exception) {
    }
    visualizer = null
  }

  private fun finished(p: MediaPlayer, error: String?) {
    if (player !== p) return
    player = null
    sounding = false
    held = false
    stopMeter()
    try {
      p.release()
    } catch (_: Exception) {
    }
    playing?.delete()
    playing = null
    emit("onPlayback", if (error == null) mapOf("type" to "end") else mapOf("type" to "error", "message" to error))
  }

  fun stopPlaying() = main.post { stopPlayingNow(true) }

  /** Pause the reply (someone may be talking over it). */
  private fun holdNow() {
    val p = player ?: return
    try {
      if (p.isPlaying) {
        p.pause()
        sounding = false
        held = true
      }
    } catch (_: Exception) {
    }
  }

  /** Carry on with a held reply: it was only noise. */
  fun resumePlaying() = main.post {
    val p = player
    if (p != null && held) {
      try {
        p.start()
        sounding = true
      } catch (_: Exception) {
      }
    }
    held = false
  }

  private fun stopPlayingNow(announce: Boolean) {
    val p = player ?: return
    player = null
    sounding = false
    held = false
    stopMeter()
    try {
      p.stop()
    } catch (_: Exception) {
    }
    try {
      p.release()
    } catch (_: Exception) {
    }
    playing?.delete()
    playing = null
    if (announce) emit("onPlayback", mapOf("type" to "stopped"))
  }

  fun destroy() = main.post {
    stopConversationNow()
    restoreBeeps()
    destroyRecognizer()
    cancelRecordingNow()
    stopPlayingNow(false)
  }
}
