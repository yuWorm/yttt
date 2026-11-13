package com.yttt.yai

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

private const val SIGN_IN_CHANNEL = "sign-in"
private const val SIGN_IN_NOTIFICATION_ID = 2

/**
 * Keeps the app's network while the browser is in front for a relay sign-in.
 * From Android 15 an app in the background reaches nothing but loopback: the
 * browser's answer arrives on the loopback port, but trading it for a session
 * at the control plane would fail. A `shortService` (Android 14+) is enough,
 * and stops itself at its time limit.
 */
class SignInService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(SIGN_IN_CHANNEL, "登录", NotificationManager.IMPORTANCE_LOW),
        )
        val notification = NotificationCompat.Builder(this, SIGN_IN_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_yttt)
            .setContentTitle("正在登录 Relay 账号")
            .setContentText("在浏览器里登录并允许后，会回到 yttt")
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
        ServiceCompat.startForeground(this, SIGN_IN_NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE)
        return START_NOT_STICKY
    }

    /** A short service must stop by its limit (about three minutes). */
    override fun onTimeout(startId: Int) {
        stopSelf()
    }

    companion object {
        fun start(context: Context) {
            if (Build.VERSION.SDK_INT >= 34) context.startForegroundService(Intent(context, SignInService::class.java))
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SignInService::class.java))
        }
    }
}
