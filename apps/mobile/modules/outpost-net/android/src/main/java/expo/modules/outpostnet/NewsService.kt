package expo.modules.outpostnet

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Instant news: the phone keeps a line open to the Outpost, so news (the doorbell, a routine's
 * result) arrives and is said the moment it's ready, not at Android's next check. Nothing
 * listens to the microphone. A quiet notification shows while it's on, with Stop.
 */
class NewsService : Service() {
  @Volatile private var running = false
  private var watcher: Thread? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      stop(this)
      return START_NOT_STICKY
    }
    try {
      show()
    } catch (_: Exception) {
      // Android won't let it start from the background (it restarted the service on its own):
      // it starts again when the app is next opened.
      stopSelf()
      return START_NOT_STICKY
    }
    alive = true
    if (watcher?.isAlive != true) {
      running = true
      watcher = Thread({ watch() }, "outpost-news").apply { start() }
    }
    return START_STICKY
  }

  override fun onDestroy() {
    alive = false
    running = false
    watcher?.interrupt()
    super.onDestroy()
  }

  private fun watch() {
    try {
      while (running) {
        // Waits up to 50 s for news; can't reach the Outpost (away from home): try again soon.
        if (JobNotifier.look(applicationContext, 50, true) == null) Thread.sleep(20_000)
      }
    } catch (_: InterruptedException) {
    } catch (_: Exception) {
    }
  }

  private fun show() {
    val manager = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Instant news", NotificationManager.IMPORTANCE_MIN).apply {
          description = "Shown while Outpost keeps a line open for the doorbell and routines"
          setShowBadge(false)
        }
      )
    }
    val openApp = packageManager.getLaunchIntentForPackage(packageName)?.let {
      PendingIntent.getActivity(this, 5, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
    val stopIt = PendingIntent.getService(
      this, 6, Intent(this, NewsService::class.java).setAction(ACTION_STOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
    )
    val n = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(JobNotifier.icon)
      .setContentTitle("Outpost")
      .setContentText("Instant news is on")
      .setOngoing(true)
      .setSilent(true)
      .setPriority(NotificationCompat.PRIORITY_MIN)
      .setContentIntent(openApp)
      .addAction(0, "Stop", stopIt)
      .build()
    val type = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0
    ServiceCompat.startForeground(this, NOTIFICATION, n, type)
  }

  companion object {
    private const val PREFS = "outpost-news"
    private const val CHANNEL = "news"
    private const val NOTIFICATION = 7302
    private const val ACTION_STOP = "expo.modules.outpostnet.NEWS_STOP"

    /** Running now (this process). */
    @Volatile private var alive = false

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** On if switched on; at first, on if the old wake word was (it kept this line open too). */
    fun on(ctx: Context): Boolean {
      val p = prefs(ctx)
      if (!p.contains("on")) {
        val was = ctx.getSharedPreferences("outpost-wake", Context.MODE_PRIVATE).getBoolean("on", false)
        p.edit().putBoolean("on", was).apply()
      }
      return p.getBoolean("on", false)
    }

    /** Switch it on (the app must be open: Android only lets it start then). */
    fun start(ctx: Context) {
      prefs(ctx).edit().putBoolean("on", true).apply()
      ContextCompat.startForegroundService(ctx, Intent(ctx, NewsService::class.java))
    }

    /** When the app opens: if it's on and not running (the phone restarted), start it again. */
    fun resume(ctx: Context) {
      if (alive || !on(ctx)) return
      ContextCompat.startForegroundService(ctx, Intent(ctx, NewsService::class.java))
    }

    fun stop(ctx: Context) {
      prefs(ctx).edit().putBoolean("on", false).apply()
      ctx.stopService(Intent(ctx, NewsService::class.java))
    }
  }
}
