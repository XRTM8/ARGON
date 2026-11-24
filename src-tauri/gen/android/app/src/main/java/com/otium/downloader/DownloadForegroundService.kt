package com.otium.downloader

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat

/**
 * Keeps active downloads alive when the user locks the screen or switches apps on Android.
 * Manages WakeLock and High-Performance WifiLock to prevent OS throttling.
 */
class DownloadForegroundService : Service() {
  private var wakeLock: PowerManager.WakeLock? = null
  private var wifiLock: WifiManager.WifiLock? = null

  companion object {
    const val CHANNEL_ID = "argon_transfers"
    const val NOTIFICATION_ID = 41523
    const val ACTION_UPDATE = "com.otium.downloader.ACTION_TRANSFER_UPDATE"
    const val ACTION_STOP = "com.otium.downloader.ACTION_TRANSFER_STOP"
    const val ACTION_PAUSE_ALL = "com.otium.downloader.ACTION_PAUSE_ALL"
    const val EXTRA_TITLE = "title"
    const val EXTRA_TEXT = "text"
    const val EXTRA_PROGRESS = "progress"
    const val EXTRA_INDETERMINATE = "indeterminate"

    fun isIgnoringBatteryOptimizations(context: Context): Boolean {
      return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
        val powerManager = context.getSystemService(Context.POWER_SERVICE) as? PowerManager
        powerManager?.isIgnoringBatteryOptimizations(context.packageName) ?: true
      } else {
        true
      }
    }

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
      releaseLocks()
      stopSelf()
      return START_NOT_STICKY
    }
    if (intent?.action == ACTION_PAUSE_ALL) {
      MainActivity.dispatchBroadcastAction("pause_all")
      releaseLocks()
      stopSelf()
      return START_NOT_STICKY
    }
    acquireLocks()
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

  override fun onDestroy() {
    releaseLocks()
    super.onDestroy()
  }

  private fun acquireLocks() {
    try {
      if (wakeLock == null) {
        val powerManager = getSystemService(Context.POWER_SERVICE) as? PowerManager
        wakeLock = powerManager?.newWakeLock(
          PowerManager.PARTIAL_WAKE_LOCK,
          "ARGON::DownloadTransferWakeLock"
        )?.apply {
          setReferenceCounted(false)
          // Safety timeout of 4 hours
          acquire(4 * 60 * 60 * 1000L)
        }
      }
      if (wifiLock == null) {
        val wifiManager = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        wifiLock = wifiManager?.createWifiLock(
          WifiManager.WIFI_MODE_FULL_HIGH_PERF,
          "ARGON::DownloadTransferWifiLock"
        )?.apply {
          setReferenceCounted(false)
          acquire()
        }
      }
    } catch (_: Exception) {}
  }

  private fun releaseLocks() {
    try {
      wakeLock?.let {
        if (it.isHeld) it.release()
      }
      wakeLock = null
      wifiLock?.let {
        if (it.isHeld) it.release()
      }
      wifiLock = null
    } catch (_: Exception) {}
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

    val pauseIntent = Intent(this, DownloadForegroundService::class.java).apply {
      action = ACTION_PAUSE_ALL
    }
    val pausePendingIntent = PendingIntent.getService(this, 1, pauseIntent, pendingFlags)

    return NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.mipmap.ic_launcher)
      .setContentTitle(title)
      .setContentText(text)
      .setContentIntent(contentIntent)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setProgress(100, progress, indeterminate)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .addAction(android.R.drawable.ic_media_pause, "Pause All", pausePendingIntent)
      .build()
  }
}
