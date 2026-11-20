package dev.priyx.pulse.update

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.content.pm.PackageManager
import android.content.pm.PackageInfo
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.pm.PackageInfoCompat
import dev.priyx.pulse.BuildConfig
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.File
import java.security.MessageDigest

/** Installs signed updates from the Pulse GitHub release channel. */
class Updater(context: Context) {

    private val app = context.applicationContext
    private val json = Json { ignoreUnknownKeys = true }
    private val client = OkHttpClient.Builder().followSslRedirects(false).build()
    private val preferences = app.getSharedPreferences("pulse_update_install", Context.MODE_PRIVATE)

    /** The last whole percent handed to the notification; see [download]. */
    private var lastReported = -1

    private val _state = MutableStateFlow<State>(State.Idle)
    val state: StateFlow<State> = _state.asStateFlow()

    sealed interface State {
        data object Idle : State
        data object Checking : State
        data object UpToDate : State
        data class Available(
            val version: String,
            val url: String,
            val bytes: Long,
            /**
             * What the release says about itself, as written on the releases
             * page.
             *
             * An update is a thing being asked for rather than announced, and
             * "there is a new version" is not enough to answer with. Empty when
             * the release carries no text, which is a release worth showing
             * anyway.
             */
            val notes: String = "",
            val sha256: String,
        ) : State
        /** 0f..1f, or null while the server sends no length to measure against. */
        data class Downloading(val progress: Float?) : State
        /** Handed to the system installer; the dialog is Android's, not ours. */
        data object Installing : State
        data class AwaitingApproval(val intent: Intent) : State
        data class Failed(val reason: String) : State
    }

    suspend fun check() {
        if (_state.value.isBusy()) return
        _state.value = State.Checking
        val result = runCatching { withContext(Dispatchers.IO) { latestRelease() } }
            .getOrElse {
                if (it is CancellationException) {
                    _state.value = State.Idle
                    throw it
                }
                android.util.Log.w(TAG, "check failed: $it")
                _state.value = State.Failed(it.message ?: "network")
                return
            }

        if (result == null || !isNewer(result.version, BuildConfig.VERSION_NAME)) {
            _state.value = State.UpToDate
            return
        }
        _state.value =
            State.Available(result.version, result.url, result.bytes, result.notes, result.sha256)
    }

    suspend fun checkAndInstall(): State.Available? {
        check()
        val update = _state.value as? State.Available ?: return null
        if (!canInstall()) {
            _state.value = State.Failed(REASON_PERMISSION)
            return update
        }
        install(update)
        return null
    }

    private data class Release(
        val version: String,
        val url: String,
        val bytes: Long,
        val notes: String,
        val sha256: String,
    )

    private fun latestRelease(): Release? {
        val request = Request.Builder()
            .url("https://api.github.com/repos/$REPO/releases/latest")
            .header("Accept", "application/vnd.github+json")
            .build()

        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) error("HTTP ${response.code}")
            val body = json.parseToJsonElement(response.body?.string().orEmpty()) as? JsonObject
                ?: return null
            val tag = body["tag_name"]?.jsonPrimitive?.content?.removePrefix("v") ?: return null
            if (!VERSION.matches(tag)) return null
            val apks = (body["assets"] as? JsonArray)
                ?.filterIsInstance<JsonObject>()
                ?.filter { it["name"]?.jsonPrimitive?.content?.endsWith(".apk") == true }
                .orEmpty()
            fun named(abi: String) = apks.firstOrNull {
                it["name"]?.jsonPrimitive?.content == "pulse-$tag-$abi.apk"
            }
            val asset = Build.SUPPORTED_ABIS.firstNotNullOfOrNull(::named)
                ?: named("universal")
                ?: return null
            val name = asset["name"]?.jsonPrimitive?.content ?: return null
            val url = asset["browser_download_url"]?.jsonPrimitive?.content ?: return null
            if (url != "https://github.com/$REPO/releases/download/v$tag/$name") return null
            val size = asset["size"]?.jsonPrimitive?.content?.toLongOrNull() ?: return null
            val hash = asset["digest"]?.jsonPrimitive?.content?.removePrefix("sha256:") ?: return null
            if (size !in 1L..MAX_APK_SIZE || !SHA256.matches(hash)) return null
            val notes = body["body"]?.jsonPrimitive?.content.orEmpty().trim()
            return Release(tag, url, size, notes, hash)
        }
    }

    fun install(update: State.Available) {
        if (_state.value.isBusy()) return
        lastOffered = update
        if (!canInstall()) {
            _state.value = State.Failed(REASON_PERMISSION)
            return
        }
        // Handed to a service rather than done here.
        //
        // This used to run in whatever scope the screen gave it, so leaving the
        // app — or merely opening the player — cancelled the download halfway
        // with nothing to show for it. See UpdateService.
        _state.value = State.Downloading(null)
        runCatching { UpdateService.start(app, update) }
            .onFailure { _state.value = State.Failed("Could not start update") }
    }

    /** Returns whether Android should retry this automatic job. */
    internal suspend fun runAutomatic(allowed: () -> Boolean): Boolean {
        if (!canAutoInstall() || !isUnmetered() || _state.value.isBusy()) return false
        if (!allowed()) return true
        check()
        val result = _state.value
        if (result is State.Failed) return result.reason != REASON_CONFIRMATION
        val update = result as? State.Available ?: return false
        if (preferences.getString(BLOCKED_VERSION, null) == update.version) {
            _state.value = State.Failed(REASON_CONFIRMATION)
            return false
        }
        if (!allowed()) return true
        lastOffered = update
        return fetchAndInstall(
            update,
            automatic = true,
            canContinue = { allowed() && canAutoInstall() && isUnmetered() },
        ) {}
    }

    internal suspend fun fetchAndInstall(
        update: State.Available,
        automatic: Boolean = false,
        canContinue: () -> Boolean = { true },
        onProgress: (Float?) -> Unit,
    ): Boolean {
        _state.value = State.Downloading(null)
        onProgress(null)
        val temporary = File(app.cacheDir, "update.apk")
        var committed = false
        try {
            withContext(Dispatchers.IO) {
                download(update, onProgress)
                currentCoroutineContext().ensureActive()
                if (!canContinue()) throw InstallDeferred()
                _state.value = State.Installing
                val coroutine = currentCoroutineContext()
                commit(temporary, update.version, automatic) { coroutine.isActive && canContinue() }
                committed = true
            }
            return false
        } catch (cancelled: CancellationException) {
            if (!committed) _state.value = State.Idle
            throw cancelled
        } catch (_: InstallDeferred) {
            _state.value = State.Idle
            return true
        } catch (invalid: InvalidApk) {
            android.util.Log.w(TAG, "Rejected update: ${invalid.message}")
            _state.value = State.Failed("Invalid update package")
            return false
        } catch (failure: Exception) {
            android.util.Log.w(TAG, "Update failed", failure)
            _state.value = State.Failed(failure.message ?: "update")
            return true
        } finally {
            temporary.delete()
        }
    }

    private class InstallDeferred : Exception()
    private class InvalidApk(message: String) : Exception(message)

    private suspend fun download(update: State.Available, onProgress: (Float?) -> Unit) {
        // cacheDir, not filesDir: once the installer has read it the file is
        // dead weight, and twenty megabytes is worth letting the system reclaim
        // if the install never happens.
        val target = File(app.cacheDir, "update.apk")
        target.delete()
        lastReported = -1

        val digest = MessageDigest.getInstance("SHA-256")
        val request = Request.Builder().url(update.url).build()
        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) error("HTTP ${response.code}")
            val body = response.body ?: error("empty body")
            val total = body.contentLength().takeIf { it > 0 } ?: update.bytes
            check(total == update.bytes) { "Unexpected APK size" }
            var written = 0L

            body.byteStream().use { input ->
                target.outputStream().use { output ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        currentCoroutineContext().ensureActive()
                        val read = input.read(buffer)
                        if (read < 0) break
                        written += read
                        check(written <= total) { "Unexpected APK size" }
                        digest.update(buffer, 0, read)
                        output.write(buffer, 0, read)
                        if (total > 0) {
                            val progress = written.toFloat() / total
                            _state.value = State.Downloading(progress)
                            // Rounded to a percent before it is reported: a
                            // notification redrawn on every sixty-fourth
                            // kilobyte is three hundred redraws for a number
                            // that changed a hundred times.
                            val step = (progress * 100).toInt()
                            if (step != lastReported) {
                                lastReported = step
                                onProgress(progress)
                            }
                        }
                    }
                }
            }
        }
        val actual = digest.digest().joinToString("") { byte -> "%02x".format(byte) }
        if (target.length() != update.bytes || actual != update.sha256) {
            throw InvalidApk("Release size or checksum mismatch")
        }
    }

    @Suppress("DEPRECATION")
    private fun verifyPackage(apk: File, version: String) {
        val modern = Build.VERSION.SDK_INT >= Build.VERSION_CODES.P
        val flags = if (modern) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
        val candidate = app.packageManager.getPackageArchiveInfo(apk.absolutePath, flags)
            ?: throw InvalidApk("Could not read package")
        if (candidate.packageName != app.packageName || candidate.versionName != version ||
            PackageInfoCompat.getLongVersionCode(candidate) <= BuildConfig.VERSION_CODE
        ) {
            throw InvalidApk("Wrong application or version")
        }
        if (signerDigests(candidate) != setOf(RELEASE_CERT)) {
            throw InvalidApk("Release signature mismatch")
        }
    }

    @Suppress("DEPRECATION")
    private fun signerDigests(info: PackageInfo): Set<String> {
        val signatures = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.signingInfo?.apkContentsSigners
        } else {
            info.signatures
        }
        return signatures.orEmpty().mapTo(mutableSetOf()) { signature ->
            MessageDigest.getInstance("SHA-256").digest(signature.toByteArray())
                .joinToString("") { byte -> "%02x".format(byte) }
        }
    }

    private fun commit(apk: File, version: String, automatic: Boolean, canContinue: () -> Boolean) {
        verifyPackage(apk, version)
        val installer = app.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(
            PackageInstaller.SessionParams.MODE_FULL_INSTALL,
        ).apply {
            setAppPackageName(app.packageName)
            setSize(apk.length())
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && automatic) {
                setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
            }
        }
        val sessionId = installer.createSession(params)
        try {
            installer.openSession(sessionId).use { session ->
                session.openWrite("pulse", 0, apk.length()).use { output ->
                    apk.inputStream().use { it.copyTo(output) }
                    session.fsync(output)
                }
                if (!canContinue()) throw InstallDeferred()
                session.commit(
                    PendingIntent.getBroadcast(
                        app,
                        sessionId,
                        Intent(app, InstallReceiver::class.java)
                            .setAction(UPDATE_RESULT)
                            .setPackage(app.packageName)
                            .putExtra(EXTRA_AUTOMATIC, automatic)
                            .putExtra(EXTRA_VERSION, version),
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
                    ).intentSender,
                )
            }
        } catch (failure: Exception) {
            runCatching { installer.abandonSession(sessionId) }
            throw failure
        }
    }

    fun canAutoInstall(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && canInstall()

    fun isUnmetered(): Boolean {
        val manager = app.getSystemService(ConnectivityManager::class.java)
        val capabilities = manager?.getNetworkCapabilities(manager.activeNetwork) ?: return false
        return capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) &&
            capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
    }

    internal fun unattendedApprovalRequired(version: String, sessionId: Int) {
        if (sessionId >= 0) {
            runCatching { app.packageManager.packageInstaller.abandonSession(sessionId) }
        }
        if (VERSION.matches(version)) preferences.edit().putString(BLOCKED_VERSION, version).apply()
        _state.value = State.Failed(REASON_CONFIRMATION)
    }

    fun awaitingApproval(intent: Intent) {
        _state.value = State.AwaitingApproval(intent)
    }

    fun approvalOpened() {
        if (_state.value is State.AwaitingApproval) _state.value = State.Installing
    }

    private fun State.isBusy() = this is State.Checking || this is State.Downloading ||
        this is State.Installing || this is State.AwaitingApproval

    /** Whether the user has allowed this app to install packages at all. */
    fun canInstall(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.O || app.packageManager.canRequestPackageInstalls()

    /**
     * The settings page where that permission is granted.
     *
     * No `NEW_TASK` flag: it is started for a result, so the caller is told when
     * the user comes back and can carry on with the install they already asked
     * for rather than making them press the row a second time.
     */
    fun permissionIntent(): Intent =
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES)
            .setData(Uri.parse("package:${app.packageName}"))

    fun dismiss() {
        _state.value = State.Idle
    }

    /** What was last sent to the installer, to offer again if it did not go in. */
    private var lastOffered: State.Available? = null

    /**
     * The installer did not install it: its dialog cancelled, or the install
     * refused.
     *
     * Offered again rather than left at [State.Installing]. Nothing else moves
     * the state on from there, so a dialog cancelled by mistake hid the update
     * until the next release came out (#151).
     */
    fun installAbandoned() {
        if (_state.value !is State.Installing && _state.value !is State.AwaitingApproval) return
        _state.value = lastOffered ?: State.Idle
    }

    companion object {
        private const val TAG = "Updater"
        private const val MAX_APK_SIZE = 128 * 1024 * 1024L
        private val VERSION = Regex("[0-9]+\\.[0-9]+\\.[0-9]+")
        private val SHA256 = Regex("[0-9a-f]{64}")

        const val REPO = "officialpriyam/pulse-yt-music-x-spotify"

        /** Told apart from a network failure so the UI can offer the way out. */
        const val REASON_PERMISSION = "permission"
        const val REASON_CONFIRMATION = "confirmation"

        internal const val UPDATE_RESULT = "dev.priyx.pulse.UPDATE_RESULT"
        internal const val EXTRA_AUTOMATIC = "dev.priyx.pulse.AUTO_UPDATE"
        internal const val EXTRA_VERSION = "dev.priyx.pulse.UPDATE_VERSION"
        private const val BLOCKED_VERSION = "confirmation_required_version"
        private const val RELEASE_CERT = "fe8d601784645cfe5288643bbfedc4dd0254c8891d49768deeebab992dd4e8cd"

        /**
         * Compares two dotted versions numerically.
         *
         * String comparison would put 1.10.0 before 1.9.0, which is exactly the
         * release where a self-updater stops offering updates and nobody
         * notices for a month.
         */
        fun isNewer(candidate: String, installed: String): Boolean {
            val a = candidate.split('.').map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
            val b = installed.split('.').map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
            for (i in 0 until maxOf(a.size, b.size)) {
                val left = a.getOrElse(i) { 0 }
                val right = b.getOrElse(i) { 0 }
                if (left != right) return left > right
            }
            return false
        }
    }
}
