package expo.modules.outpostnet

import ai.onnxruntime.OnnxTensor
import ai.onnxruntime.OrtEnvironment
import ai.onnxruntime.OrtSession
import java.nio.FloatBuffer
import kotlin.random.Random

/**
 * Hears one wake word ("Hey Jarvis") in a stream of 16 kHz audio, on the phone: openWakeWord's
 * pipeline (its models, run with ONNX Runtime).
 *
 * Every 80 ms (1280 samples): a mel spectrogram of the newest audio (8 frames of 32), an
 * embedding (96 numbers) of the last 76 frames, and the wake word's score (0..1) from the last
 * 16 embeddings (about 1.3 s). Nothing leaves the phone.
 */
class WakeWord(melModel: ByteArray, embeddingModel: ByteArray, wordModel: ByteArray) : AutoCloseable {
  private val env = OrtEnvironment.getEnvironment()
  private val options = OrtSession.SessionOptions().apply {
    setIntraOpNumThreads(1)
    setInterOpNumThreads(1)
  }
  private val mel = env.createSession(melModel, options)
  private val embedding = env.createSession(embeddingModel, options)
  private val word = env.createSession(wordModel, options)
  private val melInput = mel.inputNames.first()
  private val embeddingInput = embedding.inputNames.first()
  private val wordInput = word.inputNames.first()

  /** The newest audio: this chunk and the 480 samples before it (the spectrogram's window). */
  private val raw = FloatArray(CHUNK + OVERLAP)
  private val mels = ArrayDeque<FloatArray>()
  private val features = ArrayDeque<FloatArray>()
  private var chunks = 0

  init {
    reset()
  }

  /** Start afresh (after a wake, so the same words don't wake it twice). */
  fun reset() {
    raw.fill(0f)
    mels.clear()
    repeat(WINDOW) { mels.addLast(FloatArray(MELS) { 1f }) }
    // Embeddings of 4 s of faint noise, as openWakeWord starts.
    val rnd = Random(0)
    val noise = FloatArray(16000 * 4) { rnd.nextInt(-1000, 1000).toFloat() }
    val spec = melspectrogram(noise)
    features.clear()
    var i = 0
    while (i + WINDOW <= spec.size) {
      features.addLast(embed(spec.subList(i, i + WINDOW)))
      i += 8
    }
    while (features.size > FEATURES) features.removeFirst()
    chunks = 0
  }

  /** One 80 ms chunk (1280 samples); the wake word's score now, 0..1. */
  fun feed(pcm: ShortArray): Float {
    require(pcm.size == CHUNK) { "Feed 1280 samples at a time." }
    System.arraycopy(raw, CHUNK, raw, 0, OVERLAP)
    for (i in 0 until CHUNK) raw[OVERLAP + i] = pcm[i].toFloat()
    for (frame in melspectrogram(raw)) mels.addLast(frame)
    while (mels.size > WINDOW) mels.removeFirst()
    features.addLast(embed(mels.toList()))
    while (features.size > FEATURES) features.removeFirst()
    chunks++
    val score = score()
    // The first few chunks after a start are the noise it began with: never a wake.
    return if (chunks <= 5) 0f else score
  }

  private fun melspectrogram(audio: FloatArray): List<FloatArray> {
    OnnxTensor.createTensor(env, FloatBuffer.wrap(audio), longArrayOf(1, audio.size.toLong())).use { input ->
      mel.run(mapOf(melInput to input)).use { out ->
        val values = (out.get(0) as OnnxTensor).floatBuffer
        val frames = values.remaining() / MELS
        return List(frames) { FloatArray(MELS) { values.get() / 10f + 2f } }
      }
    }
  }

  private fun embed(window: List<FloatArray>): FloatArray {
    val flat = FloatArray(WINDOW * MELS)
    for ((f, frame) in window.withIndex()) System.arraycopy(frame, 0, flat, f * MELS, MELS)
    OnnxTensor.createTensor(env, FloatBuffer.wrap(flat), longArrayOf(1, WINDOW.toLong(), MELS.toLong(), 1)).use { input ->
      embedding.run(mapOf(embeddingInput to input)).use { out ->
        val values = (out.get(0) as OnnxTensor).floatBuffer
        return FloatArray(EMBEDDING) { values.get() }
      }
    }
  }

  private fun score(): Float {
    val flat = FloatArray(FEATURES * EMBEDDING)
    for ((i, f) in features.withIndex()) System.arraycopy(f, 0, flat, i * EMBEDDING, EMBEDDING)
    OnnxTensor.createTensor(env, FloatBuffer.wrap(flat), longArrayOf(1, FEATURES.toLong(), EMBEDDING.toLong())).use { input ->
      word.run(mapOf(wordInput to input)).use { out ->
        return (out.get(0) as OnnxTensor).floatBuffer.get()
      }
    }
  }

  override fun close() {
    word.close()
    embedding.close()
    mel.close()
    options.close()
  }

  companion object {
    const val CHUNK = 1280 // 80 ms at 16 kHz
    const val OVERLAP = 480 // the spectrogram's window reaches back 30 ms
    const val MELS = 32
    const val WINDOW = 76 // mel frames per embedding
    const val EMBEDDING = 96
    const val FEATURES = 16 // embeddings the wake word looks at

    /** The wake words that come with Outpost: id → (what to say, its model file). */
    val WORDS = linkedMapOf(
      "hey_jarvis" to ("Hey Jarvis" to "hey_jarvis_v0.1.onnx"),
      "alexa" to ("Alexa" to "alexa_v0.1.onnx"),
      "hey_mycroft" to ("Hey Mycroft" to "hey_mycroft_v0.1.onnx"),
    )

    /** Load a wake word from the app's own files. */
    fun load(context: android.content.Context, id: String): WakeWord {
      val file = WORDS[id]?.second ?: throw IllegalArgumentException("No wake word \"$id\".")
      val read = { name: String -> context.assets.open("wakeword/$name").use { it.readBytes() } }
      return WakeWord(read("melspectrogram.onnx"), read("embedding_model.onnx"), read(file))
    }

    /** The highest score over a recording (a 16 kHz, 16-bit mono WAV): for the check. */
    fun best(detector: WakeWord, wav: ByteArray): Float {
      var at = 12
      var data = 0 to 0
      while (at + 8 <= wav.size) {
        val id = String(wav, at, 4, Charsets.US_ASCII)
        val size = (wav[at + 4].toInt() and 0xff) or ((wav[at + 5].toInt() and 0xff) shl 8) or
          ((wav[at + 6].toInt() and 0xff) shl 16) or ((wav[at + 7].toInt() and 0xff) shl 24)
        if (id == "data") {
          data = (at + 8) to minOf(size, wav.size - at - 8)
          break
        }
        at += 8 + size + (size and 1)
      }
      val (start, length) = data
      val samples = ShortArray(length / 2) { i ->
        ((wav[start + 2 * i].toInt() and 0xff) or (wav[start + 2 * i + 1].toInt() shl 8)).toShort()
      }
      detector.reset()
      var best = 0f
      var i = 0
      while (i + CHUNK <= samples.size) {
        best = maxOf(best, detector.feed(samples.copyOfRange(i, i + CHUNK)))
        i += CHUNK
      }
      // Then a moment of quiet, so a word right at the end is heard.
      repeat(10) { best = maxOf(best, detector.feed(ShortArray(CHUNK))) }
      return best
    }
  }
}
