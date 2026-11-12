package dev.yttt.spike

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

private const val LINK_CHANNEL = "link"
private const val WAITING_CHANNEL = "agent-waiting"
/** The same notifications while the app is on screen: in the shade, no banner. */
private const val QUIET_WAITING_CHANNEL = "agent-waiting-quiet"
private const val ACTION_STOP = "dev.yttt.spike.STOP_LINK"

/**
 * Keeps the link with the Host while the app is in the background and posts
 * a notification when an Agent starts waiting. A `specialUse` foreground
 * service: `dataSync` is limited to six hours a day on Android 15.
 */
class LinkService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val notifier = WaitNotifier()
    private var watching: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            // The activity keeps the link while it is on screen.
            if (!link.foreground.value) link.scope.launch { link.disconnect() }
            return START_NOT_STICKY
        }
        createChannels(this)
        ServiceCompat.startForeground(
            this,
            LINK_NOTIFICATION_ID,
            ongoing(),
            if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else 0,
        )
        if (watching == null) {
            watching = scope.launch {
                link.connect()
                combine(link.overview, link.viewing, link.foreground) { overview, viewing, foreground ->
                    Triple(overview, viewing, foreground)
                }.collect { (overview, viewing, foreground) -> apply(notifier.update(overview, viewing, foreground)) }
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        scope.cancel()
        // Nothing would take them back once the service is gone.
        NotificationManagerCompat.from(this).cancelAll()
        super.onDestroy()
    }

    private fun apply(changes: List<NotificationChange>) {
        val manager = NotificationManagerCompat.from(this)
        for (change in changes) {
            when (change) {
                is NotificationChange.Cancel -> manager.cancel(WaitNotifier.notificationId(change.sessionId))
                is NotificationChange.Post -> {
                    // Denied (Android 13+): the link still works, only quietly.
                    if (Build.VERSION.SDK_INT >= 33 &&
                        ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
                        PackageManager.PERMISSION_GRANTED
                    ) {
                        continue
                    }
                    val id = WaitNotifier.notificationId(change.sessionId)
                    val open = PendingIntent.getActivity(
                        this,
                        id,
                        Intent(this, MainActivity::class.java)
                            .putExtra("open", change.sessionId)
                            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
                        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                    )
                    manager.notify(
                        id,
                        NotificationCompat.Builder(this, if (change.quiet) QUIET_WAITING_CHANNEL else WAITING_CHANNEL)
                            .setSmallIcon(android.R.drawable.stat_notify_chat)
                            .setContentTitle(change.title)
                            .setContentText(change.text)
                            .setStyle(NotificationCompat.BigTextStyle().bigText(change.text))
                            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
                            .setPriority(if (change.quiet) NotificationCompat.PRIORITY_LOW else NotificationCompat.PRIORITY_HIGH)
                            .setContentIntent(open)
                            .setAutoCancel(true)
                            .build(),
                    )
                }
            }
        }
    }

    private fun ongoing() = NotificationCompat.Builder(this, LINK_CHANNEL)
        .setSmallIcon(android.R.drawable.stat_sys_upload_done)
        .setContentTitle("保持与 Host 的连接")
        .setContentText("Agent 需要处理时会通知你")
        .setOngoing(true)
        .setContentIntent(
            PendingIntent.getActivity(
                this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
            ),
        )
        .addAction(
            0,
            "停止",
            PendingIntent.getService(
                this, 0, Intent(this, LinkService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE,
            ),
        )
        .build()

    companion object {
        fun start(context: Context) {
            ContextCompat.startForegroundService(context, Intent(context, LinkService::class.java))
        }

        fun createChannels(context: Context) {
            if (Build.VERSION.SDK_INT < 26) return
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(LINK_CHANNEL, "保持连接", NotificationManager.IMPORTANCE_LOW),
            )
            manager.createNotificationChannel(
                NotificationChannel(WAITING_CHANNEL, "Agent 需要处理", NotificationManager.IMPORTANCE_HIGH),
            )
            manager.createNotificationChannel(
                NotificationChannel(QUIET_WAITING_CHANNEL, "Agent 需要处理（App 在前台时）", NotificationManager.IMPORTANCE_LOW),
            )
        }
    }
}
