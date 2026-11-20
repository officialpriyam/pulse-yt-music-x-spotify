package dev.priyx.pulse.update

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import dev.priyx.pulse.PulseApplication
import dev.priyx.pulse.R

/** Delivers the installer result and surfaces confirmation on devices that require it. */
class InstallReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Updater.UPDATE_RESULT) return
        val updater = (context.applicationContext as PulseApplication).updater
        val automatic = intent.getBooleanExtra(Updater.EXTRA_AUTOMATIC, false)
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, -1)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                if (automatic) {
                    updater.unattendedApprovalRequired(
                        intent.getStringExtra(Updater.EXTRA_VERSION).orEmpty(),
                        intent.getIntExtra(PackageInstaller.EXTRA_SESSION_ID, -1),
                    )
                    return
                }
                val confirm = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT)
                }
                if (confirm != null) {
                    confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    updater.awaitingApproval(confirm)
                    showPrompt(context, confirm)
                } else {
                    updater.installAbandoned()
                }
            }
            PackageInstaller.STATUS_SUCCESS -> cancelPrompt(context)
            else -> {
                cancelPrompt(context)
                updater.installAbandoned()
            }
        }
    }

    private fun showPrompt(context: Context, confirm: Intent) {
        val manager = context.getSystemService(NotificationManager::class.java)
        if (!manager.areNotificationsEnabled()) return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.update_channel), NotificationManager.IMPORTANCE_DEFAULT),
        )
        val pending = PendingIntent.getActivity(
            context, PROMPT, confirm, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        manager.notify(PROMPT, Notification.Builder(context, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle(context.getString(R.string.update_approval))
            .setContentIntent(pending)
            .setAutoCancel(true)
            .build())
    }

    companion object {
        private const val CHANNEL = "pulse_update_approval"
        private const val PROMPT = 4_202

        fun cancelPrompt(context: Context) {
            context.getSystemService(NotificationManager::class.java).cancel(PROMPT)
        }
    }
}
