package dev.priyx.pulse.diagnostics

import org.junit.Assert.assertEquals
import org.junit.Test

class BackendHealthTest {
    @Test
    fun acceptsReadyPulseBackend() {
        assertEquals(
            BackendHealth.Result.Ready,
            BackendHealth.parse("""{"service":"pulse-api","status":"ok","configured":true}"""),
        )
    }

    @Test
    fun distinguishesUnconfiguredBackend() {
        assertEquals(
            BackendHealth.Result.SetupRequired,
            BackendHealth.parse("""{"service":"pulse-api","status":"ok","configured":false}"""),
        )
    }

    @Test
    fun rejectsUnrelatedOrInvalidReadinessResponses() {
        val bodies = listOf(
            """{"service":"other","status":"ok","configured":true}""",
            """{"service":"pulse-api","status":"error","configured":true}""",
            """{"service":"pulse-api","status":"ok","configured":"true"}""",
            "<html>Sign in to Vercel</html>",
        )
        bodies.forEach { body ->
            assertEquals(BackendHealth.Result.Unavailable, BackendHealth.parse(body))
        }
    }
}
