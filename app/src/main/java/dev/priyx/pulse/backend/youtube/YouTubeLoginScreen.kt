package dev.priyx.pulse.backend.youtube

import android.content.Intent
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import dev.priyx.pulse.R
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@Composable
fun YouTubeLoginScreen(account: YouTubeAccount, onDone: () -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    val scope = rememberCoroutineScope()
    var attempt by remember { mutableStateOf(account.browserAttempt) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<Int?>(null) }
    val dismiss = {
        account.browserAttempt = null
        onDone()
    }
    BackHandler(onBack = dismiss)
    fun launchBrowser() {
        if (busy) return
        scope.launch {
            busy = true
            error = null
            try {
                val login = attempt ?: account.beginBrowserSignIn().also { attempt = it }
                context.startActivity(Intent(Intent.ACTION_VIEW, login.url).addCategory(Intent.CATEGORY_BROWSABLE))
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (failure: Exception) {
                error = if (failure is BrowserGoogle.Failure && failure.status == 503) {
                    R.string.youtube_browser_unavailable
                } else {
                    R.string.youtube_sign_in_failed
                }
            } finally {
                busy = false
            }
        }
    }

    LaunchedEffect(Unit) { if (attempt == null) launchBrowser() }

    LaunchedEffect(attempt, lifecycle) {
        val login = attempt ?: return@LaunchedEffect
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (attempt == login) {
                try {
                    if (account.finishBrowserSignIn(login)) {
                        onDone()
                        break
                    }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (failure: BrowserGoogle.Failure) {
                    if (failure.status == 400 || failure.status == 403) {
                        account.browserAttempt = null
                        attempt = null
                        error = R.string.youtube_browser_expired
                        break
                    }
                    error = R.string.youtube_sign_in_failed
                } catch (_: Exception) {
                    error = R.string.youtube_sign_in_failed
                }
                delay(4_000)
            }
        }
    }

    Column(
        modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(28.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(stringResource(R.string.youtube_sign_in), style = MaterialTheme.typography.headlineSmall)
        Text(
            stringResource(if (attempt == null) R.string.youtube_browser_description else R.string.youtube_browser_waiting),
            textAlign = TextAlign.Center,
        )
        error?.let { Text(stringResource(it), color = MaterialTheme.colorScheme.error, textAlign = TextAlign.Center) }
        Button(onClick = ::launchBrowser, enabled = !busy) {
            Text(stringResource(R.string.youtube_browser_open))
        }
        TextButton(onClick = dismiss) { Text(stringResource(android.R.string.cancel)) }
    }
}
