package dev.priyx.pulse.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.priyx.pulse.R

@Composable
internal fun PulseIntro(onDone: () -> Unit) {
    val progress = remember { Animatable(0f) }
    val mint = Color(0xFF2DE1B1)
    val interaction = remember { MutableInteractionSource() }
    LaunchedEffect(Unit) {
        progress.animateTo(1f, tween(durationMillis = 1100, easing = LinearEasing))
        onDone()
    }
    BackHandler(onBack = onDone)
    Box(
        modifier = Modifier
            .fillMaxSize()
            .graphicsLayer { alpha = 1f - ((progress.value - .7f) / .3f).coerceIn(0f, 1f) }
            .background(Color(0xFF090E12))
            .clickable(interactionSource = interaction, indication = null, onClick = onDone),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.fillMaxSize()) {
            val time = progress.value
            val radius = size.minDimension * .48f
            drawCircle(
                brush = Brush.radialGradient(
                    colors = listOf(mint.copy(alpha = .19f), Color.Transparent),
                    center = center,
                    radius = radius,
                ),
                radius = radius,
            )
            repeat(3) { wave ->
                val beat = ((time - .10f * wave) / .75f).coerceIn(0f, 1f)
                if (beat > 0f) {
                    drawCircle(
                        color = mint.copy(alpha = .38f * (1f - beat)),
                        radius = radius * (.28f + beat * .9f),
                        style = Stroke(width = 1.3.dp.toPx()),
                    )
                }
            }
        }
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
            Image(
                painter = painterResource(R.mipmap.ic_launcher_foreground),
                contentDescription = null,
                modifier = Modifier
                    .size(156.dp)
                    .graphicsLayer {
                        val entrance = (progress.value / .34f).coerceIn(0f, 1f)
                        alpha = entrance
                        scaleX = .76f + entrance * .24f
                        scaleY = scaleX
                    },
            )
            Text(
                "Pulse",
                color = Color.White,
                fontSize = 30.sp,
                fontWeight = FontWeight.SemiBold,
                letterSpacing = 5.sp,
                modifier = Modifier.graphicsLayer {
                    alpha = ((progress.value - .24f) / .26f).coerceIn(0f, 1f)
                },
            )
        }
    }
}
