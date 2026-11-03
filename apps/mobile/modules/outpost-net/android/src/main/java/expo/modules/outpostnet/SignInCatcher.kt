package expo.modules.outpostnet

import android.content.Context
import android.content.Intent
import java.net.InetAddress
import java.net.ServerSocket
import java.net.SocketTimeoutException

/**
 * Signing in to a service from the phone: the browser is sent back to
 * http://127.0.0.1:<port>/callback, which this catches (the same way the desktop app does),
 * then brings Outpost back to the front. Only this phone can reach 127.0.0.1.
 */
object SignInCatcher {
  @Volatile private var socket: ServerSocket? = null

  private const val DONE =
    "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width'><title>Signed in</title>" +
      "<body style=\"font-family:system-ui;background:#16110f;color:#f3eee8;display:grid;place-items:center;height:100vh;margin:0;text-align:center\">" +
      "<div><h1>Back to Outpost</h1><p>Outpost is finishing the sign-in. You can close this tab.</p></div>"

  /** Start listening on 127.0.0.1:port. False when the port is taken. */
  fun listen(port: Int): Boolean {
    stop()
    return try {
      socket = ServerSocket(port, 4, InetAddress.getByName("127.0.0.1"))
      true
    } catch (e: Exception) {
      false
    }
  }

  /** Wait (on a background thread) for the browser's /callback; returns its path and query. */
  fun await(context: Context, timeoutMs: Int): String {
    val server = socket ?: throw IllegalStateException("Not listening for a sign-in.")
    server.soTimeout = timeoutMs
    try {
      while (true) {
        val sock = try {
          server.accept()
        } catch (e: SocketTimeoutException) {
          throw IllegalStateException("The sign-in took too long. Try again.")
        }
        sock.use { s ->
          // Browsers open spare connections that never send anything: don't wait on those.
          s.soTimeout = 3000
          val line = try {
            s.getInputStream().bufferedReader().readLine() ?: ""
          } catch (e: SocketTimeoutException) {
            ""
          }
          val target = line.split(" ").getOrNull(1) ?: ""
          val out = s.getOutputStream()
          if (!target.startsWith("/callback")) {
            out.write("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".toByteArray())
            out.flush()
            return@use
          }
          val body = DONE.toByteArray()
          out.write(
            ("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: ${body.size}\r\n" +
              "Connection: close\r\n\r\n").toByteArray()
          )
          out.write(body)
          out.flush()
          bringToFront(context)
          return target
        }
      }
    } finally {
      stop()
    }
  }

  fun stop() {
    try {
      socket?.close()
    } catch (_: Exception) {
    }
    socket = null
  }

  private fun bringToFront(context: Context) {
    val intent = context.packageManager.getLaunchIntentForPackage(context.packageName) ?: return
    intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_REORDER_TO_FRONT)
    try {
      context.startActivity(intent)
    } catch (_: Exception) {
      // Android may not let a background app come forward; the person switches back.
    }
  }
}
