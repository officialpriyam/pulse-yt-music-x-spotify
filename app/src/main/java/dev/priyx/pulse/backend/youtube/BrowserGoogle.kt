package dev.priyx.pulse.backend.youtube

import android.net.Uri
import android.util.Base64
import dev.priyx.pulse.BuildConfig
import dev.priyx.pulse.data.CatalogPlaylist
import dev.priyx.pulse.data.CatalogTrack
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.io.IOException
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.TimeUnit

internal class BrowserGoogle {
    data class Attempt(val state: String, val verifier: String, val url: Uri)
    data class Session(val token: String, val name: String)
    class Failure(val status: Int) : IOException("Pulse API returned HTTP $status")

    private val client = OkHttpClient.Builder()
        .followRedirects(false)
        .callTimeout(15, TimeUnit.SECONDS)
        .build()

    suspend fun begin(): Attempt {
        val verifier = encode(ByteArray(32).also(SecureRandom()::nextBytes))
        val challenge = encode(MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray()))
        val data = request("/api/v1/android/google/start", body = JSONObject().put("challenge", challenge)).second
        val browserUrl = Uri.parse(data.getString("browser_url"))
        require(browserUrl.scheme == "https" && !browserUrl.host.isNullOrBlank() &&
            browserUrl.path == "/auth/android/google/start" && browserUrl.userInfo == null)
        return Attempt(data.getString("state"), verifier, browserUrl)
    }

    suspend fun complete(attempt: Attempt): Session? {
        val (status, data) = request(
            "/api/v1/android/google/complete",
            body = JSONObject().put("state", attempt.state).put("verifier", attempt.verifier),
        )
        return if (status == 202) null else Session(data.getString("token"), data.optString("name"))
    }

    suspend fun connected(token: String): Boolean =
        request("/api/v1/session", token).second.optBoolean("youtube_connected")

    suspend fun logout(token: String) {
        request("/auth/logout", token, JSONObject())
    }

    suspend fun playlists(token: String): List<CatalogPlaylist> =
        pages("/api/v1/youtube/playlists", token) { item ->
            val id = item.optString("id")
            val snippet = item.optJSONObject("snippet") ?: return@pages null
            if (id.isEmpty()) return@pages null
            CatalogPlaylist(
                uri = "${YouTubeBackend.CONNECTED_PLAYLIST_PREFIX}$id",
                name = snippet.getString("title"),
                artworkUrl = artwork(snippet),
            )
        }

    suspend fun tracks(token: String, playlist: String?): List<CatalogTrack> =
        pages(
            if (playlist == null) "/api/v1/youtube/liked" else "/api/v1/youtube/playlist-items",
            token,
            playlist,
        ) { item ->
            val snippet = item.optJSONObject("snippet") ?: return@pages null
            val id = item.optJSONObject("contentDetails")?.optString("videoId")?.takeIf { it.isNotEmpty() }
                ?: snippet.optJSONObject("resourceId")?.optString("videoId")?.takeIf { it.isNotEmpty() }
                ?: item.optString("id")
            val title = snippet.optString("title")
            if (!VIDEO_ID.matches(id) || title.isBlank() || title == "Private video" || title == "Deleted video") {
                return@pages null
            }
            CatalogTrack(
                uri = "${YouTubeBackend.TRACK_PREFIX}$id",
                name = title,
                artist = snippet.optString("videoOwnerChannelTitle").ifBlank { snippet.optString("channelTitle") },
                artworkUrl = artwork(snippet) ?: "https://i.ytimg.com/vi/$id/mqdefault.jpg",
            )
        }

    private suspend fun <T> pages(
        path: String,
        token: String,
        playlist: String? = null,
        parse: (JSONObject) -> T?,
    ): List<T> = buildList {
        var next: String? = null
        val cursors = mutableSetOf<String>()
        repeat(20) {
            val data = request(path, token, playlist = playlist, pageToken = next).second
            val items = data.optJSONArray("items")
            if (items != null) {
                for (index in 0 until items.length()) {
                    items.optJSONObject(index)?.let(parse)?.let(::add)
                }
            }
            next = data.optString("nextPageToken").takeIf { it.isNotEmpty() && cursors.add(it) }
            if (next == null) return@buildList
        }
    }

    private suspend fun request(
        path: String,
        token: String? = null,
        body: JSONObject? = null,
        playlist: String? = null,
        pageToken: String? = null,
    ): Pair<Int, JSONObject> = withContext(Dispatchers.IO) {
        val url = "${BuildConfig.PULSE_BACKEND_URL}$path".toHttpUrl().newBuilder().apply {
            playlist?.let { addQueryParameter("playlist_id", it) }
            pageToken?.let { addQueryParameter("page_token", it) }
        }.build()
        val request = Request.Builder().url(url).header("X-Pulse-Request", "1").apply {
            token?.let { header("Authorization", "Bearer $it") }
            body?.let { post(it.toString().toRequestBody("application/json".toMediaType())) }
        }.build()
        client.newCall(request).execute().use { response ->
            if (!response.isSuccessful) throw Failure(response.code)
            val content = response.body?.string().orEmpty()
            response.code to if (content.isBlank()) JSONObject() else JSONObject(content)
        }
    }

    private fun encode(bytes: ByteArray): String =
        Base64.encodeToString(bytes, Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)

    private fun artwork(snippet: JSONObject): String? {
        val thumbnails = snippet.optJSONObject("thumbnails") ?: return null
        return listOf("high", "medium", "default").firstNotNullOfOrNull { size ->
            thumbnails.optJSONObject(size)?.optString("url")?.takeIf { it.startsWith("https://") }
        }
    }

    private companion object {
        val VIDEO_ID = Regex("[A-Za-z0-9_-]{11}")
    }
}
