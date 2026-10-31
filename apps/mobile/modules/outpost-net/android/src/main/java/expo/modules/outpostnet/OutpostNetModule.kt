package expo.modules.outpostnet

import android.Manifest
import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import android.util.Base64
import expo.modules.interfaces.permissions.PermissionsResponseListener
import expo.modules.interfaces.permissions.PermissionsStatus
import expo.modules.kotlin.Promise
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import expo.modules.kotlin.records.Field
import expo.modules.kotlin.records.Record
import okhttp3.Call
import okhttp3.Callback
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.asRequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.IOException
import java.net.InetSocketAddress
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

/** One HTTPS request to the paired Outpost. */
class RequestOptions : Record {
  @Field val url: String = ""
  @Field val method: String = "GET"
  @Field val headers: Map<String, String> = emptyMap()
  @Field val body: String? = null
  /** SHA-256 of the Outpost's certificate (hex): the only certificate accepted. */
  @Field val fingerprint: String = ""
  /** Return the body as base64 (pictures). */
  @Field val base64: Boolean = false
  @Field val timeoutMs: Int = 30000
}

/** Marks an activity's start intent as already taken (so a share isn't handled twice). */
private const val TAKEN = "outpost.shareTaken"

private fun sha256Hex(bytes: ByteArray): String =
  MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

/**
 * Trusts exactly one certificate: the Outpost's own, by its SHA-256. Outpost makes its
 * certificate itself, so no certificate authority vouches for it; pinning is the check.
 */
private class PinnedTrust(private val pin: String) : X509TrustManager {
  override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
    throw CertificateException("Client certificates aren't used.")
  }

  override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
    val leaf = chain.firstOrNull() ?: throw CertificateException("No certificate.")
    if (!sha256Hex(leaf.encoded).equals(pin, ignoreCase = true)) {
      throw CertificateException("This isn't the Outpost you paired with: its certificate is different.")
    }
  }

  override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
}

/** Accepts any certificate and remembers it. Only used to read a fingerprint before pairing. */
private class Capture : X509TrustManager {
  @Volatile var seen: X509Certificate? = null

  override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
    throw CertificateException("Client certificates aren't used.")
  }

  override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
    seen = chain.firstOrNull()
  }

  override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
}

class OutpostNetModule : Module() {
  private val clients = ConcurrentHashMap<String, OkHttpClient>()
  private val streams = ConcurrentHashMap<String, Call>()

  private var nsd: NsdManager? = null
  private var discovery: NsdManager.DiscoveryListener? = null
  private var multicast: WifiManager.MulticastLock? = null
  private val toResolve = ConcurrentLinkedQueue<NsdServiceInfo>()
  private val resolving = AtomicBoolean(false)

  private val context: Context
    get() = appContext.reactContext ?: throw IllegalStateException("No Android context.")

  /** A share that arrived while Outpost was running, until the app takes it. */
  @Volatile private var pendingShare: android.content.Intent? = null

  private val updater by lazy { AppUpdater({ context }) { name, body -> sendEvent(name, body) } }
  private val voice by lazy { PhoneVoice({ context }) { name, body -> sendEvent(name, body) } }

  private fun client(fingerprint: String): OkHttpClient =
    clients.getOrPut(fingerprint.lowercase()) {
      val trust = PinnedTrust(fingerprint)
      val tls = SSLContext.getInstance("TLS")
      tls.init(null, arrayOf(trust), SecureRandom())
      OkHttpClient.Builder()
        .sslSocketFactory(tls.socketFactory, trust)
        // The pin above is the identity check; the address may change (DHCP), so names aren't checked.
        .hostnameVerifier { _, _ -> true }
        // Short, so an address that isn't reachable (home Wi-Fi while away) gives way
        // quickly to the next one (Tailscale).
        .connectTimeout(4, TimeUnit.SECONDS)
        .build()
    }

  private fun build(o: RequestOptions): Request {
    val method = o.method.uppercase()
    val json = "application/json".toMediaType()
    val text = o.body
    val body = when {
      text != null -> text.toRequestBody(json)
      method == "POST" || method == "PUT" || method == "PATCH" -> "".toRequestBody(json)
      else -> null
    }
    val b = Request.Builder().url(o.url).method(method, body)
    for ((k, v) in o.headers) b.header(k, v)
    return b.build()
  }

  override fun definition() = ModuleDefinition {
    Name("OutpostNet")

    Events("onChunk", "onEnd", "onError", "onService", "onServiceLost", "onSpeech", "onRecording", "onPlayback", "onConverse", "onUpdate", "onShare")

    // ------------------------------------------------------------ shared with Outpost (see ShareReceiver)

    /** What was shared with Outpost (or an Outpost link opened), once; null if nothing. */
    AsyncFunction("takeShared") {
      val intent = pendingShare ?: appContext.currentActivity?.intent?.takeIf { !it.getBooleanExtra(TAKEN, false) }
      pendingShare = null
      // Read once: the activity keeps its intent, so mark it.
      appContext.currentActivity?.intent?.putExtra(TAKEN, true)
      ShareReceiver.read(context, intent)
    }

    /** Delete the copies of shared files. */
    Function("clearShared") { ShareReceiver.clear(context) }

    // ------------------------------------------------------------ saving to the phone (see Saving)

    /** Copy a downloaded file into the phone's Pictures, Movies or Download folder (Outpost). */
    AsyncFunction("saveToPhone") { path: String, name: String, mime: String ->
      Saving.save(context, path, name, mime).toString()
    }

    /** The share sheet for a file saved with saveToPhone. */
    AsyncFunction("shareSaved") { uri: String, mime: String, title: String ->
      val activity = appContext.currentActivity ?: throw IllegalStateException("Outpost isn't open.")
      Saving.share(activity, android.net.Uri.parse(uri), mime, title)
    }

    /** Opened again with a share (or link) while running: the app is told, and takes it. */
    OnNewIntent { intent ->
      if (intent.action == android.content.Intent.ACTION_SEND ||
        intent.action == android.content.Intent.ACTION_SEND_MULTIPLE ||
        intent.action == android.content.Intent.ACTION_VIEW
      ) {
        pendingShare = intent
        sendEvent("onShare", mapOf<String, Any?>())
      }
    }

    /** Send a file from the phone as a request's body (documents, pictures for the gallery). */
    AsyncFunction("upload") { o: RequestOptions, path: String, contentType: String, promise: Promise ->
      val file = java.io.File(path.removePrefix("file://"))
      if (!file.isFile) {
        promise.reject("ERR_FILE", "That file is gone.", null)
        return@AsyncFunction
      }
      val body = file.asRequestBody(contentType.toMediaType())
      val b = Request.Builder().url(o.url).post(body)
      for ((k, v) in o.headers) b.header(k, v)
      client(o.fingerprint).newBuilder()
        .writeTimeout(10, TimeUnit.MINUTES)
        .readTimeout(2, TimeUnit.MINUTES)
        .build()
        .newCall(b.build())
        .enqueue(object : Callback {
          override fun onFailure(call: Call, e: IOException) {
            promise.reject("ERR_NETWORK", e.message ?: "Can't reach Outpost.", e)
          }

          override fun onResponse(call: Call, response: Response) {
            response.use {
              promise.resolve(
                mapOf("status" to it.code, "body" to (it.body?.string() ?: ""), "contentType" to (it.header("Content-Type") ?: "")),
              )
            }
          }
        })
    }

    // ------------------------------------------------------------ updating the app (see AppUpdater)

    Function("updateStatus") { updater.status() }
    Function("openInstallSettings") { updater.openInstallSettings(); Unit }
    Function("updateDownloaded") { name: String -> updater.downloaded(name) }

    AsyncFunction("fetchText") { url: String, promise: Promise ->
      Thread {
        try {
          promise.resolve(updater.fetchText(url))
        } catch (e: Exception) {
          promise.reject("ERR_NETWORK", e.message ?: "Couldn't reach GitHub.", e)
        }
      }.start()
    }

    AsyncFunction("downloadUpdate") { url: String, name: String, sha256: String?, promise: Promise ->
      Thread {
        try {
          promise.resolve(updater.download(url, name, sha256))
        } catch (e: Exception) {
          promise.reject("ERR_UPDATE", e.message ?: "The update couldn't be downloaded.", e)
        }
      }.start()
    }

    AsyncFunction("installUpdate") { path: String, promise: Promise ->
      Thread {
        try {
          updater.install(path)
          promise.resolve(null)
        } catch (e: Exception) {
          promise.reject("ERR_UPDATE", e.message ?: "The update couldn't be installed.", e)
        }
      }.start()
    }

    // ------------------------------------------------------------ voice (see PhoneVoice)

    Function("voiceSupport") { voice.support() }

    /** Ask for the microphone; resolves true when allowed. */
    AsyncFunction("askMicrophone") { promise: Promise ->
      val permissions = appContext.permissions
      if (permissions == null) {
        promise.resolve(false)
      } else {
        permissions.askForPermissions(
          PermissionsResponseListener { result ->
            promise.resolve(result[Manifest.permission.RECORD_AUDIO]?.status == PermissionsStatus.GRANTED)
          },
          Manifest.permission.RECORD_AUDIO
        )
      }
    }

    Function("listen") { language: String?, partial: Boolean -> voice.listen(language, partial); Unit }
    Function("stopListening") { voice.stopListening(); Unit }
    Function("cancelListening") { voice.cancelListening(); Unit }
    Function("record") { autoStop: Boolean -> voice.record(autoStop); Unit }
    Function("stopRecording") { voice.stopRecording(); Unit }
    Function("cancelRecording") { voice.cancelRecording(); Unit }
    Function("play") { uri: String -> voice.play(uri); Unit }
    Function("stopPlaying") { voice.stopPlaying(); Unit }
    Function("resumePlaying") { voice.resumePlaying(); Unit }
    Function("converse") { bargeIn: Boolean -> voice.converse(bargeIn); Unit }
    Function("converseMuted") { muted: Boolean -> voice.converseMuted(muted); Unit }
    Function("stopConversation") { voice.stopConversation(); Unit }

    /** The SHA-256 of the certificate an address presents, before anything is trusted. */
    AsyncFunction("fingerprint") { host: String, port: Int, promise: Promise ->
      Thread {
        try {
          val capture = Capture()
          val tls = SSLContext.getInstance("TLS")
          tls.init(null, arrayOf(capture), SecureRandom())
          val socket = tls.socketFactory.createSocket() as SSLSocket
          socket.use {
            it.soTimeout = 8000
            it.connect(InetSocketAddress(host, port), 5000)
            it.startHandshake()
          }
          val cert = capture.seen
          if (cert == null) promise.reject("ERR_NO_CERT", "That address didn't present a certificate.", null)
          else promise.resolve(sha256Hex(cert.encoded))
        } catch (e: Exception) {
          promise.reject("ERR_NETWORK", "Can't reach $host:$port (${e.message ?: e.javaClass.simpleName}).", e)
        }
      }.start()
    }

    /** One request; resolves with status, body (text or base64) and content type. */
    AsyncFunction("request") { o: RequestOptions, promise: Promise ->
      val call = client(o.fingerprint).newBuilder()
        .callTimeout(o.timeoutMs.toLong(), TimeUnit.MILLISECONDS)
        .build()
        .newCall(build(o))
      call.enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) {
          promise.reject("ERR_NETWORK", e.message ?: "Can't reach Outpost.", e)
        }

        override fun onResponse(call: Call, response: Response) {
          response.use {
            try {
              val bytes = it.body?.bytes() ?: ByteArray(0)
              val text = if (o.base64) Base64.encodeToString(bytes, Base64.NO_WRAP) else String(bytes, Charsets.UTF_8)
              promise.resolve(
                mapOf(
                  "status" to it.code,
                  "body" to text,
                  "contentType" to (it.header("Content-Type") ?: "")
                )
              )
            } catch (e: Exception) {
              promise.reject("ERR_NETWORK", e.message ?: "The reply was cut off.", e)
            }
          }
        }
      })
    }

    /** Save a reply to a file in the app's cache (videos, which the player reads from disk). */
    AsyncFunction("download") { o: RequestOptions, name: String, promise: Promise ->
      val safe = name.replace(Regex("[^A-Za-z0-9._-]"), "_").take(80)
      val dir = java.io.File(context.cacheDir, "outpost-media").apply { mkdirs() }
      val file = java.io.File(dir, safe)
      if (file.isFile && file.length() > 0) {
        promise.resolve("file://" + file.absolutePath)
      } else {
      val call = client(o.fingerprint).newBuilder()
        .readTimeout(5, TimeUnit.MINUTES)
        .build()
        .newCall(build(o))
      call.enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) {
          promise.reject("ERR_NETWORK", e.message ?: "Can't reach Outpost.", e)
        }

        override fun onResponse(call: Call, response: Response) {
          response.use {
            if (!it.isSuccessful) {
              promise.reject("ERR_HTTP", "Outpost said no (${it.code}).", null)
              return
            }
            try {
              val part = java.io.File(dir, "$safe.part")
              part.outputStream().use { out -> it.body?.byteStream()?.copyTo(out) }
              part.renameTo(file)
              promise.resolve("file://" + file.absolutePath)
            } catch (e: Exception) {
              promise.reject("ERR_NETWORK", e.message ?: "The download was cut off.", e)
            }
          }
        }
      })
      }
    }

    /** A streamed reply (Server-Sent Events): each line arrives as an onChunk event. */
    Function("stream") { id: String, o: RequestOptions ->
      val call = client(o.fingerprint).newBuilder()
        .readTimeout(10, TimeUnit.MINUTES)
        .build()
        .newCall(build(o))
      streams[id] = call
      call.enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) {
          streams.remove(id)
          if (!call.isCanceled()) sendEvent("onError", mapOf("id" to id, "message" to (e.message ?: "Can't reach Outpost.")))
          else sendEvent("onEnd", mapOf("id" to id, "status" to 0, "body" to ""))
        }

        override fun onResponse(call: Call, response: Response) {
          response.use {
            try {
              if (!it.isSuccessful) {
                sendEvent("onEnd", mapOf("id" to id, "status" to it.code, "body" to (it.body?.string() ?: "")))
                return
              }
              val source = it.body?.source()
              while (source != null && !call.isCanceled()) {
                val line = source.readUtf8Line() ?: break
                sendEvent("onChunk", mapOf("id" to id, "line" to line))
              }
              sendEvent("onEnd", mapOf("id" to id, "status" to it.code, "body" to ""))
            } catch (e: Exception) {
              if (call.isCanceled()) sendEvent("onEnd", mapOf("id" to id, "status" to 0, "body" to ""))
              else sendEvent("onError", mapOf("id" to id, "message" to (e.message ?: "The reply was cut off.")))
            } finally {
              streams.remove(id)
            }
          }
        }
      })
    }

    Function("cancel") { id: String ->
      streams.remove(id)?.cancel()
    }

    /** Look for Outposts on the Wi-Fi; each one arrives as an onService event. */
    Function("startDiscovery") {
      stopDiscovery()
      val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
      multicast = wifi?.createMulticastLock("outpost-discovery")?.apply {
        setReferenceCounted(false)
        acquire()
      }
      val manager = context.getSystemService(Context.NSD_SERVICE) as NsdManager
      nsd = manager
      val listener = object : NsdManager.DiscoveryListener {
        override fun onDiscoveryStarted(serviceType: String) {}
        override fun onDiscoveryStopped(serviceType: String) {}
        override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
          sendEvent("onError", mapOf("id" to "discovery", "message" to "Couldn't search the network ($errorCode)."))
        }
        override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
        override fun onServiceFound(info: NsdServiceInfo) {
          toResolve.add(info)
          resolveNext()
        }
        override fun onServiceLost(info: NsdServiceInfo) {
          sendEvent("onServiceLost", mapOf("name" to info.serviceName))
        }
      }
      discovery = listener
      manager.discoverServices("_outpost._tcp", NsdManager.PROTOCOL_DNS_SD, listener)
    }

    Function("stopDiscovery") {
      stopDiscovery()
    }

    OnDestroy {
      stopDiscovery()
      voice.destroy()
      for (call in streams.values) call.cancel()
    }
  }

  private fun stopDiscovery() {
    val listener = discovery
    discovery = null
    if (listener != null) {
      try {
        nsd?.stopServiceDiscovery(listener)
      } catch (_: Exception) {
      }
    }
    toResolve.clear()
    resolving.set(false)
    try {
      multicast?.release()
    } catch (_: Exception) {
    }
    multicast = null
  }

  /** Android resolves one service at a time, so found services wait their turn. */
  @Suppress("DEPRECATION")
  private fun resolveNext() {
    val manager = nsd ?: return
    if (!resolving.compareAndSet(false, true)) return
    val next = toResolve.poll()
    if (next == null) {
      resolving.set(false)
      return
    }
    manager.resolveService(next, object : NsdManager.ResolveListener {
      override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {
        resolving.set(false)
        resolveNext()
      }

      override fun onServiceResolved(info: NsdServiceInfo) {
        val txt = info.attributes.mapValues { (_, v) -> v?.let { String(it, Charsets.UTF_8) } ?: "" }
        sendEvent(
          "onService",
          mapOf(
            "name" to info.serviceName,
            "host" to (info.host?.hostAddress ?: ""),
            "port" to info.port,
            "txt" to txt
          )
        )
        resolving.set(false)
        resolveNext()
      }
    })
  }
}
