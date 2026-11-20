package dev.priyx.pulse.playback

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.media3.common.C
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import dev.priyx.pulse.backend.youtube.YouTubeBackend
import dev.priyx.pulse.data.DownloadSettingsStore
import dev.priyx.pulse.download.YouTubeDownloads
import dev.priyx.pulse.nativecore.NativeBridge
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/** Warms only the next few tracks of the local playback queue. */
@UnstableApi
internal class UpcomingCache(
    private val context: Context,
    settings: DownloadSettingsStore,
    scope: CoroutineScope,
) {
    private data class Window(val current: String?, val upcoming: List<String>)
    private val window = MutableStateFlow(Window(null, emptyList()))
    private val connectivity = requireNotNull(context.getSystemService(ConnectivityManager::class.java))
    private val networks = callbackFlow {
        fun publish() {
            val caps = connectivity.getNetworkCapabilities(connectivity.activeNetwork)
            trySend(Link(
                online = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) == true,
                metered = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) != true,
            ))
        }
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = publish()
            override fun onLost(network: Network) = publish()
            override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = publish()
        }
        connectivity.registerDefaultNetworkCallback(callback)
        publish()
        awaitClose { connectivity.unregisterNetworkCallback(callback) }
    }.distinctUntilChanged()

    private data class Link(val online: Boolean, val metered: Boolean)

    init {
        scope.launch(Dispatchers.IO) {
            val allowed = combine(settings.cacheUpcoming, settings.wifiOnly, OfflineMode.active, networks) { enabled, wifi, offline, link ->
                enabled && !offline && link.online && (!wifi || !link.metered)
            }.distinctUntilChanged()
            combine(window, allowed, settings.cacheUpcoming) { list, online, enabled ->
                Triple(list, online, enabled)
            }.collectLatest { (list, online, enabled) ->
                val retain = if (enabled) list.upcoming.toSet() else emptySet()
                // A track that's playing from disk may still need another range.
                YouTubeDownloads.retainUpcoming(context, retain + listOfNotNull(list.current))
                if (!online || !enabled || list.upcoming.isEmpty()) return@collectLatest
                delay(5_000)
                var warmingSpotify = false
                for (uri in list.upcoming) {
                    try {
                        if (YouTubeBackend.isSong(uri)) {
                            YouTubeDownloads.cacheUpcoming(context, uri)
                        } else if (uri.startsWith("spotify:track:")) {
                            if (warmingSpotify) delay(4_000)
                            warmingSpotify = true
                            if (!NativeBridge.warmTrack(uri)) break
                        }
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (_: Exception) {
                        // A missing or oversized track doesn't stop the rest of the queue.
                    }
                }
            }
        }
    }

    fun update(player: Player, eligible: Boolean) {
        val timeline = player.currentTimeline
        val current = player.currentMediaItemIndex
        val currentUri = player.currentMediaItem?.mediaId
        if (timeline.isEmpty || !eligible || !player.playWhenReady) {
            // Pausing keeps the same ready tracks for a later resume or a network loss.
            if (currentUri != window.value.current || !eligible) {
                window.value = Window(currentUri, emptyList())
            }
            return
        }
        val repeatMode = if (player.repeatMode == Player.REPEAT_MODE_ALL) Player.REPEAT_MODE_ALL else Player.REPEAT_MODE_OFF
        val upcoming = buildList {
            var next = current
            repeat(3) {
                next = timeline.getNextWindowIndex(next, repeatMode, player.shuffleModeEnabled)
                if (next == C.INDEX_UNSET || next == current || next >= player.mediaItemCount) return@buildList
                val uri = player.getMediaItemAt(next).mediaId
                if (uri != currentUri && uri !in this) add(uri)
            }
        }
        window.value = Window(currentUri, upcoming)
    }
}
