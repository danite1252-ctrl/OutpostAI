package expo.modules.outpostnet

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * While jobs of yours are waiting or running, the phone keeps a line open to the Outpost
 * (asking it to answer the moment one finishes), so "it's ready" arrives straight away even
 * with the app closed. A quiet notification says it's watching; it ends when the jobs do
 * (or after six hours).
 */
class JobWatchService : Service() {
  @Volatile private var worker: Thread? = null

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    show(null)
    if (worker?.isAlive != true) {
      worker = Thread({ watch() }, "outpost-job-watch").apply { start() }
    }
    return START_NOT_STICKY
  }

  private fun watch() {
    val started = System.currentTimeMillis()
    var misses = 0
    try {
      while (System.currentTimeMillis() - started < 6 * 60 * 60 * 1000L) {
        val left = JobNotifier.look(this, 45)
        if (left == null) {
          // Can't reach the Outpost (out of range for a moment): try again, then give up.
          if (++misses > 30) break
          Thread.sleep(10_000)
          continue
        }
        misses = 0
        if (left == 0) break
        show(left)
      }
    } catch (_: InterruptedException) {
    } finally {
      ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
      stopSelf()
    }
  }

  private fun show(left: Int?) {
    val manager = getSystemService(NotificationManager::class.java)
    if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Waiting for jobs", NotificationManager.IMPORTANCE_MIN).apply {
          description = "Shown while Outpost waits for your jobs to finish, so it can tell you straight away"
          setShowBadge(false)
        }
      )
    }
    val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
      PendingIntent.getActivity(this, 1, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }
    val text = when (left) {
      null -> "Waiting for your jobs to finish"
      1 -> "Waiting for 1 job to finish"
      else -> "Waiting for $left jobs to finish"
    }
    val n = NotificationCompat.Builder(this, CHANNEL)
      .setSmallIcon(JobNotifier.icon)
      .setContentTitle("Outpost")
      .setContentText(text)
      .setOngoing(true)
      .setSilent(true)
      .setPriority(NotificationCompat.PRIORITY_MIN)
      .apply { if (open != null) setContentIntent(open) }
      .build()
    try {
      ServiceCompat.startForeground(
        this,
        ID,
        n,
        if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC else 0
      )
    } catch (_: Exception) {
      // Android refused (started from the background): the regular checks cover it.
      stopSelf()
    }
  }

  override fun onDestroy() {
    worker?.interrupt()
    super.onDestroy()
  }

  companion object {
    private const val CHANNEL = "watching"
    private const val ID = 4207
  }
}
