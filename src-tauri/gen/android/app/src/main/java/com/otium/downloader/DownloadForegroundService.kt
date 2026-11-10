package com.otium.downloader

import android.app.Notification
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

/**
 * Keeps active downloads alive when the user locks the screen or switches apps on Android.
 */
class DownloadForegroundService : Service() {
  companion object {
    const val CHANNEL_ID = "argon_transfers"
    const val NOTIFICATION_ID = 41523
    const val ACTION_UPDATE = "com.otium.downloader.ACTION_TRANSFER_UPDATE"
    const val ACTION_STOP = "com.otium.downloader.ACTION_TRANSFER_STOP"
    const val EXTRA_TITLE = "title"
    const val EXTRA_TEXT = "text"
    const val EXTRA_PROGRESS = "progress"
    const val EXTRA_INDETERMINATE = "indeterminate"

    fun syncState(
      context: Context,
      activeCount: Int,
      title: String,
      text: String,
      progress: Int,
      indeterminate: Boolean
    ) {
      if (activeCount <= 0) {
        context.stopService(Intent(context, DownloadForegroundService::class.java))
        return
      }
      val intent = Intent(context, DownloadForegroundService::class.java).apply {
        action = ACTION_UPDATE
        putExtra(EXTRA_TITLE, title)
        putExtra(EXTRA_TEXT, text)
        putExtra(EXTRA_PROGRESS, progress.coerceIn(0, 100))
        putExtra(EXTRA_INDETERMINATE, indeterminate)
      }
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        context.startForegroundService(intent)
      } else {
        context.startService(intent)
      }
    }
  }

  override fun onCreate() {
    super.onCreate()
    ensureChannel()
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    if (intent?.action == ACTION_STOP) {
      stopSelf()
      return START_NOT_STICKY
    }
    val title = intent?.getStringExtra(EXTRA_TITLE) ?: "ARGON"
    val text = intent?.getStringExtra(EXTRA_TEXT) ?: "Downloading..."
    val progress = intent?.getIntExtra(EXTRA_PROGRESS, 0) ?: 0
    val indeterminate = intent?.getBooleanExtra(EXTRA_INDETERMINATE, true) ?: true

    val notification = buildNotification(title, text, progress, indeterminate)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
      startForeground(
        NOTIFICATION_ID,
        notification,
        ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
      )
    } else {
      startForeground(NOTIFICATION_ID, notification)
    }
    return START_NOT_STICKY
  }

  override fun onBind(intent: Intent?): IBinder? = null

  private fun ensureChannel() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      val existing = manager.getNotificationChannel(CHANNEL_ID)
      if (existing == null) {
        val channel = NotificationChannel(
          CHANNEL_ID,
          "Active Downloads",
          NotificationManager.IMPORTANCE_LOW
        ).apply {
          description = "Shows progress for ongoing ARGON downloads"
          setShowBadge(false)
        }
        manager.createNotificationChannel(channel)
      }
    }
  }

  private fun buildNotification(
    title: String,
    text: String,
    progress: Int,
    indeterminate: Boolean
  ): Notification {
    val openIntent = Intent(this, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    val pendingFlags = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    } else {
      PendingIntent.FLAG_UPDATE_CURRENT
    }
    val contentIntent = PendingIntent.getActivity(this, 0, openIntent, pendingFlags)

    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle(title)
      .setContentText(text)
      .setContentIntent(contentIntent)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setProgress(100, progress, indeterminate)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .build()
  }
}
