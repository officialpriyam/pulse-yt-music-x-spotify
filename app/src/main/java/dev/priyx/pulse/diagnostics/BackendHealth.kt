package dev.priyx.pulse.diagnostics

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.OkHttpClient
import okhttp3.Request
import org.json.JSONException
import org.json.JSONObject

internal object BackendHealth {
    enum class Result {
        Ready,
        SetupRequired,
        Unavailable,
    }

    suspend fun check(origin: String, client: OkHttpClient): Result = withContext(Dispatchers.IO) {
        val request = Request.Builder()
            .url("$origin/healthz")
            .header("Accept", "application/json")
            .build()
        val probe = client.newBuilder()
            .callTimeout(8, TimeUnit.SECONDS)
            .followRedirects(false)
            .followSslRedirects(false)
            .build()

        try {
            probe.newCall(request).execute().use { response ->
                if (response.code != 200 && response.code != 503) return@use Result.Unavailable
                parse(response.peekBody(4096).string(), response.code)
            }
        } catch (_: IOException) {
            Result.Unavailable
        }
    }

    internal fun parse(body: String, status: Int = 200): Result {
        val health = try {
            JSONObject(body)
        } catch (_: JSONException) {
            return Result.Unavailable
        }
        if (health.optString("service") != "pulse-api") return Result.Unavailable
        if (status == 503 && health.optString("status") == "configuration_required" &&
            health.opt("configured") == false
        ) {
            return Result.SetupRequired
        }
        if (status != 200 || health.optString("status") != "ok") return Result.Unavailable
        return when (health.opt("configured")) {
            true -> Result.Ready
            false -> Result.SetupRequired
            else -> Result.Unavailable
        }
    }
}
