package dev.priyx.pulse.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.draw.drawBehind
import androidx.compose.material3.Icon
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.priyx.pulse.R
import dev.priyx.pulse.ui.theme.Ink

/** Pulse drawn with square caps on the same grid as the app titles. */
@Composable
fun PulseWordmark(
    height: Dp,
    modifier: Modifier = Modifier,
    color: Color = Ink,
) {
    Canvas(
        modifier
            .height(height)
            // Each glyph advances 9.5 units in a 12-unit-high box.
            .width(height * (GLYPHS.size * 9.5f / 12f)),
    ) {
        val unit = size.height / 12f
        // The stroke stays near one unit and the advance well over the glyph
        // width, or the counters close up and the word turns into a dark block.
        val stroke = unit * 1.15f
        val path = Path()
        GLYPHS.forEachIndexed { index, lines ->
            val originX = index * 9.5f * unit + stroke / 2f
            lines.forEach { points ->
                points.forEachIndexed { at, (gx, gy) ->
                    val x = originX + gx * unit
                    val y = gy * unit + stroke / 2f
                    if (at == 0) path.moveTo(x, y) else path.lineTo(x, y)
                }
            }
        }
        drawPath(
            path,
            color = color,
            style = Stroke(width = stroke, cap = StrokeCap.Square, join = StrokeJoin.Miter),
        )
    }
}

/**
 * The launcher art beside the name.
 *
 * The real icon rather than a drawing of one: this is what the user tapped to
 * get here, and showing the same thing is what says the two are one app.
 */
@Composable
fun AppLockup(
    modifier: Modifier = Modifier,
    iconSize: Dp = 28.dp,
    nameHeight: Dp = 14.dp,
    /**
     * What to put the mark on, when it should sit on glass.
     *
     * Null leaves it bare, which is what everywhere but the home header wants:
     * a plate is a control, and a mark in the middle of a login page is not one.
     */
    plate: dev.priyx.pulse.ui.glass.backdrop.Backdrop? = null,
) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        if (plate == null) {
            AppGlyph(iconSize)
        } else {
            AppPlate(iconSize, plate)
        }
        Spacer(Modifier.width(iconSize * 0.32f))
        PulseWordmark(nameHeight)
    }
}

/**
 * The mark on a round pane of the app's own glass.
 *
 * The same material and the same rim as the round buttons — see CircleAction —
 * so it belongs to the same row of controls even though there is nothing to
 * press. What it gives the mark is a place to stand.
 *
 * With something behind it to bend. Glass over the page's own flat colour has
 * nothing to refract and comes out as a grey disc, so the pane is given a wash
 * of the accent the app takes from the artwork that is playing: off-centre, so
 * the film and the rim have a gradient to sit on, and alive, because that
 * colour slides to a new one every time the music changes.
 *
 * Flat, deliberately. This sits in a header that moves with the scroll, and a
 * pane that samples the screen re-photographs it every frame — for a
 * reflection of a colour this already knows.
 */
@Composable
fun AppPlate(
    size: Dp,
    backdrop: dev.priyx.pulse.ui.glass.backdrop.Backdrop,
    modifier: Modifier = Modifier,
) {
    val accent = androidx.compose.material3.MaterialTheme.colorScheme.primary
    // The iOS 27 glass, its own edge and light, over the artwork behind the
    // page: that layer stands still while the page scrolls, so the pane is
    // not taken again on every frame of it.
    dev.priyx.pulse.ui.glass.LiquidButton(
        onClick = {},
        backdrop = backdrop,
        isInteractive = false,
        modifier = modifier.size(size),
        contentHeight = size,
        contentPadding = 0.dp,
    ) {
        Box(Modifier.size(size), contentAlignment = Alignment.Center) {
            // A veil of the accent the app takes from the artwork playing,
            // light enough that the glass is what reads: it slides to a new
            // colour every time the music changes.
            androidx.compose.foundation.Canvas(Modifier.matchParentSize()) {
                drawCircle(
                    androidx.compose.ui.graphics.Brush.linearGradient(
                        0f to accent.copy(alpha = 0.34f),
                        1f to accent.copy(alpha = 0f),
                        start = androidx.compose.ui.geometry.Offset.Zero,
                        end = androidx.compose.ui.geometry.Offset(this.size.width, this.size.height),
                    ),
                )
            }
            AppGlyph(size * 0.58f)
        }
    }
}

/** The Pulse mark tinted to match the surrounding UI. */
@Composable
fun AppGlyph(size: Dp, modifier: Modifier = Modifier, tint: Color = Ink) {
    Icon(
        painter = painterResource(R.drawable.ic_app_mark),
        contentDescription = null,
        tint = tint,
        modifier = modifier.size(size),
    )
}

/** P U L S E, as polylines on a 6 × 10 grid. */
private val GLYPHS = "PULSE".map { TITLE_GLYPHS.getValue(it) }
