package com.andymitch.vellum

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

// Foreground service that keeps the process — and with it the in-process iroh
// node — alive while Vellum is backgrounded, so this device stays a reachable
// sync peer (hub mode). Without it, Android freezes/kills the process and sync
// only happens while the app is in the foreground. Started/stopped from
// MainActivity.setBackgroundSync, which Rust calls for the Background sync toggle.
class SyncService : Service() {
  override fun onBind(intent: Intent?): IBinder? = null

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    val notification = buildNotification()
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      startForeground(NOTIF_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
    } else {
      startForeground(NOTIF_ID, notification)
    }
    // Come back if the OS kills us while backgrounded.
    return START_STICKY
  }

  private fun buildNotification(): Notification {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      val mgr = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
      val ch = NotificationChannel(CHANNEL, "Background sync", NotificationManager.IMPORTANCE_LOW)
      ch.description = "Keeps your notes syncing while Vellum is in the background."
      mgr.createNotificationChannel(ch)
    }
    return NotificationCompat.Builder(this, CHANNEL)
      .setContentTitle("Vellum")
      .setContentText("Syncing notes in the background")
      .setSmallIcon(R.mipmap.ic_launcher)
      .setOngoing(true)
      .setPriority(NotificationCompat.PRIORITY_LOW)
      .build()
  }

  companion object {
    private const val CHANNEL = "vellum_sync"
    private const val NOTIF_ID = 1

    fun start(ctx: Context) {
      val intent = Intent(ctx, SyncService::class.java)
      if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
        ctx.startForegroundService(intent)
      } else {
        ctx.startService(intent)
      }
    }

    fun stop(ctx: Context) {
      ctx.stopService(Intent(ctx, SyncService::class.java))
    }
  }
}
