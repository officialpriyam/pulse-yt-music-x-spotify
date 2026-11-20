package dev.priyx.pulse.backend.youtube

import android.content.Context
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import dev.priyx.pulse.R
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import com.metrolist.innertube.YouTube
import com.metrolist.innertube.models.YouTubeChannel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Google-backed YouTube access, with support for existing InnerTube sessions. */
class YouTubeAccount(private val context: Context) {
    private val browserGoogle = BrowserGoogle()
    internal var browserAttempt: BrowserGoogle.Attempt? = null

    private val prefs = EncryptedSharedPreferences.create(
        context,
        FILE_NAME,
        MasterKey.Builder(context).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    /** The display name of the signed-in account, or null when signed out. */
    private val _accountName = MutableStateFlow(prefs.getString(KEY_NAME, null))
    val accountName: StateFlow<String?> = _accountName.asStateFlow()

    private val mobileToken: String? get() = prefs.getString(KEY_MOBILE_TOKEN, null)
    internal val hasMobileSession: Boolean get() = mobileToken != null
    internal val hasLegacySession: Boolean get() = prefs.getString(KEY_COOKIE, null) != null
    val isSignedIn: Boolean get() = hasMobileSession || hasLegacySession

    internal suspend fun beginBrowserSignIn(): BrowserGoogle.Attempt =
        browserGoogle.begin().also { browserAttempt = it }

    internal suspend fun finishBrowserSignIn(attempt: BrowserGoogle.Attempt): Boolean {
        val session = browserGoogle.complete(attempt) ?: return false
        if (browserAttempt != attempt) return false
        val name = session.name.ifBlank { context.getString(R.string.youtube_account_fallback_name) }
        withContext(Dispatchers.IO) {
            check(prefs.edit().clear().putString(KEY_MOBILE_TOKEN, session.token).putString(KEY_NAME, name).commit())
        }
        browserAttempt = null
        _pageId.value = null
        _expired.value = false
        _accountName.value = name
        applyToInnerTube()
        return true
    }

    internal suspend fun connectedPlaylists() = browserGoogle.playlists(checkNotNull(mobileToken))

    internal suspend fun connectedTracks(playlist: String?) =
        browserGoogle.tracks(checkNotNull(mobileToken), playlist)

    internal fun likedPlaylistName(): String = context.getString(R.string.liked_songs)

    /**
     * True once the stored session has been refused.
     *
     * A cookie that has stopped working looks exactly like an empty library
     * from the outside, which is the worst way for this to fail: nothing on
     * screen says the account came loose. This is what lets a screen say it.
     */
    private val _expired = MutableStateFlow(false)
    val expired: StateFlow<Boolean> = _expired.asStateFlow()

    init {
        // Before anything asks the module for a page: the cookie is what turns
        // its calls from "the anonymous catalogue" into "this account's".
        applyToInnerTube()
    }

    /**
     * Hands the stored session to the InnerTube client.
     *
     * The three values travel together and are only meaningful together:
     * `visitorData` and `dataSyncId` identify which of a Google account's
     * several YouTube channels is being asked about, and a cookie without them
     * reads the wrong one.
     */
    private fun applyToInnerTube() {
        YouTube.cookie = prefs.getString(KEY_COOKIE, null)
        YouTube.visitorData = prefs.getString(KEY_VISITOR_DATA, null).orEmpty()
        YouTube.dataSyncId = prefs.getString(KEY_DATA_SYNC_ID, null).orEmpty()
        // Which of the account's channels is answering. Null is the personal
        // one, which is what the cookie alone reads as.
        YouTube.pageId = prefs.getString(KEY_PAGE_ID, null)
        // Browsing as the account too, not only the library. InnerTube sends
        // the cookie with a browse only when told to, and nothing told it: the
        // home page was the one anybody in the country gets, with none of the
        // account's own shelves (what it played lately, the mixes and albums
        // made for it) that YouTube Music shows the same account.
        YouTube.useLoginForBrowse = YouTube.cookie != null
    }

    /**
     * The channels this Google account can act as.
     *
     * A Google account is not one YouTube identity: there is the personal
     * channel and any brand channel it owns — a channel with its own
     * subscriptions, its own library, its own history. The official app has a
     * switcher for exactly this, and until now a session here always answered
     * as whichever one Google considered active.
     */
    suspend fun channels(): List<YouTubeChannel> =
        if (!hasLegacySession) emptyList() else YouTube.accounts().getOrElse {
            android.util.Log.w(TAG, "channel list unavailable: $it")
            emptyList()
        }

    /** Which one is being read, by page id; null is the personal channel. */
    private val _pageId = MutableStateFlow(prefs.getString(KEY_PAGE_ID, null))
    val pageId: StateFlow<String?> = _pageId.asStateFlow()

    /**
     * Reads the account as this channel from now on.
     *
     * The page id and the datasync id go together: the first says which
     * channel the request is for, the second which library to sync it against,
     * and a mismatched pair reads the right name over the wrong music.
     */
    suspend fun useChannel(channel: YouTubeChannel): Boolean {
        val previousPageId = prefs.getString(KEY_PAGE_ID, null)
        val previousSyncId = prefs.getString(KEY_DATA_SYNC_ID, null)
        val previousName = prefs.getString(KEY_NAME, null)

        // Lengths, not values: these two are what let a request act as this
        // channel, and a log is not the place for them. Whether they are there
        // at all is the whole question when a switch is refused.
        android.util.Log.i(
            TAG,
            "switching channel: pageId=${channel.pageId?.length ?: 0} chars," +
                " dataSyncId=${channel.dataSyncId?.length ?: 0} chars",
        )

        applyChannel(channel.pageId, channel.dataSyncId.orEmpty(), channel.name)

        // Tried before it is believed. The two tokens have to be the pair
        // Google issued together, and a mismatched one is refused — which used
        // to leave the app claiming the session had expired, when the session
        // was fine and it was this switch that was not.
        val attempt = YouTube.accountInfo()
        val worked = attempt.isSuccess
        if (!worked) {
            // The message, not just the outcome: "refused" covers a wrong token
            // pair, a client that does not accept the header, and a session
            // that was already gone, and those want three different answers.
            android.util.Log.w(
                TAG,
                "channel refused: ${attempt.exceptionOrNull()?.message?.take(400)}",
            )
            android.util.Log.w(TAG, "channel refused, going back to the previous one")
            applyChannel(previousPageId, previousSyncId.orEmpty(), previousName.orEmpty())
        }
        return worked
    }

    private fun applyChannel(pageId: String?, dataSyncId: String, name: String) {
        prefs.edit()
            .putString(KEY_PAGE_ID, pageId)
            .putString(KEY_DATA_SYNC_ID, dataSyncId)
            .putString(KEY_NAME, name)
            .apply()
        _pageId.value = pageId
        _accountName.value = name
        applyToInnerTube()
    }

    /** Recheck the active transport while keeping sessions across transient outages. */
    suspend fun revalidate() {
        mobileToken?.let { token ->
            try {
                _expired.value = !browserGoogle.connected(token)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: BrowserGoogle.Failure) {
                if (failure.status == 401 || failure.status == 403) _expired.value = true
            } catch (_: Exception) {
                // Keep the active session on temporary network failures.
            }
            return
        }
        if (!hasLegacySession) return

        // The cookie is left exactly as the sign-in left it.
        //
        // This used to be copied from the web view's store on every check, on
        // the theory that Google renews a session there and the saved copy goes
        // stale. It cost this account its session twice: the web view collects
        // cookies from any page of music.youtube.com it loads, and those are
        // not the signed-in session — the first time they lacked SAPISID
        // entirely, the second they carried one that answered 401. A saved
        // session that Google still accepts must not be replaced by a guess
        // about a better one.

        if (YouTube.visitorData.isNullOrBlank()) {
            YouTube.visitorData()
                .onSuccess { fresh ->
                    prefs.edit().putString(KEY_VISITOR_DATA, fresh).apply()
                    YouTube.visitorData = fresh
                }
        }

        YouTube.accountInfo()
            .onSuccess { info ->
                _expired.value = false
                if (info.name != _accountName.value) {
                    prefs.edit().putString(KEY_NAME, info.name).apply()
                    _accountName.value = info.name
                }
            }
            .onFailure {
                // Only a refusal means the session is done. A phone with no
                // network would otherwise sign the user out for being on a
                // train.
                val refused = it.message.orEmpty().let { message ->
                    "401" in message || "403" in message
                }
                android.util.Log.w(TAG, "youtube session check failed (refused=$refused): $it")
                _expired.value = refused
            }
    }

    suspend fun signOut() {
        val token = mobileToken
        prefs.edit().clear().apply()
        browserAttempt = null
        _accountName.value = null
        _pageId.value = null
        _expired.value = false
        applyToInnerTube()
        if (token != null) {
            try {
                browserGoogle.logout(token)
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // The device has already forgotten the credential.
            }
        }
    }

    private companion object {
        const val FILE_NAME = "pulse_youtube_account"
        const val KEY_COOKIE = "cookie"
        const val KEY_MOBILE_TOKEN = "mobile_token"
        const val KEY_VISITOR_DATA = "visitor_data"
        const val KEY_DATA_SYNC_ID = "data_sync_id"
        const val KEY_NAME = "name"
        const val KEY_PAGE_ID = "page_id"
        const val TAG = "PulseYouTubeAccount"
    }
}
