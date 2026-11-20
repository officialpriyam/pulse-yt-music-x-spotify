package dev.priyx.pulse.update

import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.media.AudioManager
import android.os.Build
import dev.priyx.pulse.BuildConfig
import dev.priyx.pulse.PulseApplication
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import java.util.concurrent.TimeUnit

/** Runs an unattended update only when Android schedules idle, unmetered work. */
class AutomaticUpdateJob : JobService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    private val running = mutableMapOf<Int, Job>()

    override fun onStartJob(params: JobParameters): Boolean {
        val app = application as PulseApplication
        val audio = getSystemService(AudioManager::class.java)
        running[params.jobId] = scope.launch {
            val retry = app.updater.runAutomatic {
                app.preferences.automaticUpdates.value && audio?.isMusicActive == false
            }
            running.remove(params.jobId)
            jobFinished(params, retry)
        }
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean {
        running.remove(params.jobId)?.cancel()
        return true
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        private const val JOB_ID = 42103
        private const val FIRST_JOB_ID = 42104

        fun configure(context: Context, enabled: Boolean) {
            val scheduler = context.getSystemService(JobScheduler::class.java) ?: return
            if (!enabled || BuildConfig.BUILD_TYPE != "release" || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) {
                scheduler.cancel(JOB_ID)
                scheduler.cancel(FIRST_JOB_ID)
                return
            }
            if (scheduler.getPendingJob(JOB_ID) != null) return
            val periodic = builder(context, JOB_ID)
                .setPeriodic(TimeUnit.HOURS.toMillis(6))
                .build()
            if (scheduler.schedule(periodic) == JobScheduler.RESULT_SUCCESS) request(context)
        }

        fun request(context: Context) {
            if (BuildConfig.BUILD_TYPE != "release" || Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
            val scheduler = context.getSystemService(JobScheduler::class.java) ?: return
            if (scheduler.getPendingJob(FIRST_JOB_ID) == null) {
                scheduler.schedule(builder(context, FIRST_JOB_ID).build())
            }
        }

        private fun builder(context: Context, id: Int): JobInfo.Builder =
            JobInfo.Builder(id, ComponentName(context, AutomaticUpdateJob::class.java))
                .setRequiredNetworkType(JobInfo.NETWORK_TYPE_UNMETERED)
                .setRequiresDeviceIdle(true)
                .setPersisted(true)
    }
}
