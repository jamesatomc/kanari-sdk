package com.jamesatomc.kanariapp.ui.components

import androidx.compose.animation.core.*
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import com.jamesatomc.kanariapp.ui.theme.KanariColors
import kotlin.math.*

private val Lime = KanariColors.Lime
private val Purple = KanariColors.Purple
private val Lavender = KanariColors.Lavender
private val Ink = KanariColors.Ink
private val Cream = KanariColors.Cream

@Composable
fun IsometricWalletIllustration(
    modifier: Modifier = Modifier,
    shadowColor: Color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.2f)
) {
    val infiniteTransition = rememberInfiniteTransition(label = "walletAnim")
    val floatY by infiniteTransition.animateFloat(
        initialValue = 0f, targetValue = -12f,
        animationSpec = infiniteRepeatable(tween(2500, easing = EaseInOutSine), RepeatMode.Reverse),
        label = "floatY"
    )
    val sweepPos by infiniteTransition.animateFloat(
        initialValue = -0.5f, targetValue = 1.5f,
        animationSpec = infiniteRepeatable(tween(3000, easing = LinearEasing)),
        label = "sweep"
    )
    val coinFloat1 by infiniteTransition.animateFloat(
        initialValue = 0f, targetValue = -8f,
        animationSpec = infiniteRepeatable(tween(1800, easing = EaseInOutSine), RepeatMode.Reverse),
        label = "coin1"
    )
    val coinFloat2 by infiniteTransition.animateFloat(
        initialValue = -6f, targetValue = 4f,
        animationSpec = infiniteRepeatable(tween(2200, easing = EaseInOutSine), RepeatMode.Reverse),
        label = "coin2"
    )
    val glowAlpha by infiniteTransition.animateFloat(
        initialValue = 0.3f, targetValue = 0.6f,
        animationSpec = infiniteRepeatable(tween(2000), RepeatMode.Reverse),
        label = "glow"
    )

    Canvas(modifier = modifier) {
        val w = size.width
        val h = size.height
        val cx = w / 2f
        val cy = h / 2f

        drawCircle(
            brush = Brush.radialGradient(
                colors = listOf(Lime.copy(alpha = glowAlpha * 0.3f), Color.Transparent),
                center = Offset(cx, cy + floatY), radius = w * 0.4f
            ),
            radius = w * 0.4f, center = Offset(cx, cy + floatY)
        )

        val walletW = w * 0.38f
        val walletH = w * 0.28f
        val walletX = cx - walletW / 2f
        val walletY = cy - walletH / 2f + floatY

        drawRoundRect(
            color = shadowColor, topLeft = Offset(walletX, walletY + 4.dp.toPx()),
            size = Size(walletW, walletH), cornerRadius = CornerRadius(12.dp.toPx())
        )

        // Main wallet body with sweep effect
        val walletBrush = Brush.linearGradient(
            colors = listOf(Purple, Lavender, Purple),
            start = Offset(walletX, walletY),
            end = Offset(walletX + walletW, walletY + walletH)
        )

        drawRoundRect(
            brush = walletBrush,
            topLeft = Offset(walletX, walletY),
            size = Size(walletW, walletH),
            cornerRadius = CornerRadius(12.dp.toPx())
        )

        // Light sweep overlay
        drawRoundRect(
            brush = Brush.linearGradient(
                0.0f to Color.Transparent,
                0.5f to Color.White.copy(alpha = 0.2f),
                1.0f to Color.Transparent,
                start = Offset(walletX + (walletW * sweepPos), walletY),
                end = Offset(walletX + (walletW * sweepPos) + 40.dp.toPx(), walletY + walletH)
            ),
            topLeft = Offset(walletX, walletY),
            size = Size(walletW, walletH),
            cornerRadius = CornerRadius(12.dp.toPx()),
            blendMode = BlendMode.Overlay
        )

        drawRoundRect(
            color = Lime,
            topLeft = Offset(walletX + walletW * 0.3f, walletY + walletH * 0.35f),
            size = Size(walletW * 0.4f, walletH * 0.08f),
            cornerRadius = CornerRadius(4.dp.toPx())
        )
        drawCircle(
            color = Lime,
            radius = walletH * 0.15f,
            center = Offset(walletX + walletW * 0.72f, walletY + walletH * 0.39f)
        )

        data class Coin(val baseX: Float, val baseY: Float, val r: Float, val floatOffset: Float, val color: Color)

        val coins = listOf(
            Coin(cx - walletW * 0.55f, walletY - 10f, 12f, coinFloat1, Lime),
            Coin(cx + walletW * 0.5f, walletY - 5f, 10f, coinFloat2, Lavender),
            Coin(cx - walletW * 0.35f, walletY + walletH + 8f, 8f, coinFloat2, Purple),
            Coin(cx + walletW * 0.4f, walletY + walletH + 12f, 11f, coinFloat1, Lime.copy(alpha = 0.7f)),
            Coin(cx + walletW * 0.15f, walletY - 18f, 7f, coinFloat1, Cream)
        )
        coins.forEach { c ->
            drawCircle(
                brush = Brush.radialGradient(
                    colors = listOf(c.color, c.color.copy(alpha = 0.4f)),
                    center = Offset(c.baseX, c.baseY + c.floatOffset),
                    radius = c.r
                ),
                radius = c.r,
                center = Offset(c.baseX, c.baseY + c.floatOffset)
            )
            drawCircle(
                color = Color.White.copy(alpha = 0.4f),
                radius = c.r * 0.3f,
                center = Offset(c.baseX - c.r * 0.2f, c.baseY + c.floatOffset - c.r * 0.2f)
            )
        }
    }
}

@Composable
fun IsometricNetworkOrbit(
    modifier: Modifier = Modifier,
    labelColor: Color = MaterialTheme.colorScheme.onSurface
) {
    val infiniteTransition = rememberInfiniteTransition(label = "orbitAnim")
    val rotation by infiniteTransition.animateFloat(
        initialValue = 0f, targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(26000, easing = LinearEasing)),
        label = "rotation"
    )
    val pulse by infiniteTransition.animateFloat(
        initialValue = 0.8f, targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(1500, easing = EaseInOutSine), RepeatMode.Reverse),
        label = "pulse"
    )

    Canvas(modifier = modifier) {
        val w = size.width
        val h = size.height
        val cx = w / 2f
        val cy = h / 2f
        val orbitRadius = w * 0.34f

        // Multiple orbits for depth
        drawCircle(
            color = Lavender.copy(alpha = 0.05f),
            radius = orbitRadius + 20.dp.toPx(),
            center = Offset(cx, cy)
        )
        drawCircle(
            color = Lavender.copy(alpha = 0.12f),
            radius = orbitRadius,
            center = Offset(cx, cy),
            style = Stroke(
                width = 1.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(12f, 8f))
            )
        )

        val centerR = 20.dp.toPx() * pulse
        drawCircle(
            brush = Brush.radialGradient(
                listOf(Lime, Purple),
                center = Offset(cx, cy),
                radius = centerR
            ),
            radius = centerR,
            center = Offset(cx, cy)
        )
        drawCircle(color = Ink.copy(alpha = 0.8f), radius = centerR * 0.5f, center = Offset(cx, cy))

        val nodeLabels = listOf("K", "PQ", "01", "TX", "SC", "M")
        val nodeColors = listOf(Lime, Lavender, Purple, Lime.copy(alpha = 0.7f), Cream, Purple.copy(alpha = 0.6f))
        val nodeR = 12.dp.toPx()

        nodeLabels.forEachIndexed { i, label ->
            val angle = Math.toRadians((rotation + i * (360.0 / nodeLabels.size)).toDouble())
            val nx = cx + orbitRadius * cos(angle).toFloat()
            val ny = cy + orbitRadius * sin(angle).toFloat()

            // Connecting line to center
            drawLine(
                color = nodeColors[i].copy(alpha = 0.2f),
                start = Offset(cx, cy),
                end = Offset(nx, ny),
                strokeWidth = 1.dp.toPx()
            )

            drawCircle(
                brush = Brush.radialGradient(
                    colors = listOf(nodeColors[i], nodeColors[i].copy(alpha = 0.3f)),
                    center = Offset(nx, ny), radius = nodeR
                ),
                radius = nodeR, center = Offset(nx, ny)
            )

            // Node glow
            drawCircle(
                brush = Brush.radialGradient(
                    colors = listOf(nodeColors[i].copy(alpha = 0.4f), Color.Transparent),
                    center = Offset(nx, ny), radius = nodeR * 2f
                ),
                radius = nodeR * 2f, center = Offset(nx, ny)
            )

            drawContext.canvas.nativeCanvas.apply {
                val paint = android.graphics.Paint().apply {
                    color = labelColor.toArgb()
                    textSize = 9.dp.toPx()
                    textAlign = android.graphics.Paint.Align.CENTER
                    typeface = android.graphics.Typeface.DEFAULT_BOLD
                }
                drawText(label, nx, ny + 3.5.dp.toPx(), paint)
            }
        }
    }
}

@Composable
fun IsometricShieldLock(
    modifier: Modifier = Modifier,
    shadowColor: Color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.2f)
) {
    val infiniteTransition = rememberInfiniteTransition()
    val floatY by infiniteTransition.animateFloat(
        initialValue = 0f, targetValue = -6f,
        animationSpec = infiniteRepeatable(tween(2200, easing = EaseInOutSine), RepeatMode.Reverse)
    )
    val glowAlpha by infiniteTransition.animateFloat(
        initialValue = 0.2f, targetValue = 0.5f,
        animationSpec = infiniteRepeatable(tween(1800), RepeatMode.Reverse)
    )

    Canvas(modifier = modifier) {
        val w = size.width
        val h = size.height
        val cx = w / 2f
        val cy = h / 2f

        drawCircle(
            brush = Brush.radialGradient(
                colors = listOf(Lime.copy(alpha = glowAlpha * 0.3f), Color.Transparent),
                center = Offset(cx, cy + floatY), radius = w * 0.35f
            ),
            radius = w * 0.35f, center = Offset(cx, cy + floatY)
        )

        val shieldW = w * 0.45f
        val shieldH = w * 0.5f
        val shieldX = cx - shieldW / 2f
        val shieldY = cy - shieldH / 2f + floatY

        val shieldPath = Path().apply {
            moveTo(shieldX + shieldW / 2f, shieldY)
            lineTo(shieldX + shieldW, shieldY + shieldH * 0.25f)
            lineTo(shieldX + shieldW * 0.9f, shieldY + shieldH * 0.7f)
            quadraticBezierTo(
                shieldX + shieldW / 2f, shieldY + shieldH * 1.05f,
                shieldX + shieldW * 0.1f, shieldY + shieldH * 0.7f
            )
            lineTo(shieldX, shieldY + shieldH * 0.25f)
            close()
        }

        drawPath(
            path = shieldPath,
            brush = Brush.linearGradient(
                colors = listOf(Purple, Purple.copy(alpha = 0.6f)),
                start = Offset(shieldX, shieldY),
                end = Offset(shieldX + shieldW, shieldY + shieldH)
            )
        )

        val lockW = shieldW * 0.3f
        val lockH = shieldH * 0.22f
        val lockX = cx - lockW / 2f
        val lockY = cy - lockH / 2f + floatY + shieldH * 0.05f

        drawRoundRect(
            color = Lime,
            topLeft = Offset(lockX, lockY + lockH * 0.35f),
            size = Size(lockW, lockH * 0.65f),
            cornerRadius = CornerRadius(3.dp.toPx())
        )
        drawArc(
            color = Lime,
            startAngle = 180f, sweepAngle = 180f, useCenter = false,
            topLeft = Offset(lockX + lockW * 0.15f, lockY),
            size = Size(lockW * 0.7f, lockH * 0.7f),
            style = Stroke(width = 3.dp.toPx(), cap = StrokeCap.Round)
        )

        drawCircle(
            color = shadowColor,
            radius = 2.5.dp.toPx(),
            center = Offset(cx, lockY + lockH * 0.6f)
        )
    }
}

@Composable
fun IsometricCoinStack(modifier: Modifier = Modifier) {
    val infiniteTransition = rememberInfiniteTransition(label = "coinStack")
    val floatY by infiniteTransition.animateFloat(
        initialValue = 0f, targetValue = -8f,
        animationSpec = infiniteRepeatable(tween(2000, easing = EaseInOutSine), RepeatMode.Reverse),
        label = "float"
    )

    Canvas(modifier = modifier) {
        val w = size.width
        val h = size.height
        val cx = w / 2f
        val cy = h / 2f

        val coinW = w * 0.32f
        val coinH = w * 0.1f
        val coinRadius = coinH / 2f
        val spacing = coinH * 0.7f
        val stackCount = 6

        for (i in 0 until stackCount) {
            val offsetX = (stackCount - 1 - i) * 2.5.dp.toPx()
            val x = cx - coinW / 2f + offsetX
            val yBase = cy + (stackCount - 1 - i) * spacing - stackCount * spacing / 2f
            val y = yBase + floatY * (1f - i * 0.1f)

            // Individual coin shadow
            drawRoundRect(
                color = Color.Black.copy(alpha = 0.15f),
                topLeft = Offset(x, y + 2.dp.toPx()),
                size = Size(coinW, coinH),
                cornerRadius = CornerRadius(coinRadius)
            )

            val colors = listOf(
                listOf(Lime, Lime.copy(alpha = 0.8f)),
                listOf(Purple, Purple.copy(alpha = 0.8f)),
                listOf(Lavender, Lavender.copy(alpha = 0.8f)),
                listOf(Lime, Purple.copy(alpha = 0.6f)),
                listOf(Cream, Lime.copy(alpha = 0.5f)),
                listOf(Purple, Lavender)
            )
            val c = colors[i % colors.size]

            drawRoundRect(
                brush = Brush.linearGradient(
                    colors = c, start = Offset(x, y), end = Offset(x + coinW, y + coinH)
                ),
                topLeft = Offset(x, y),
                size = Size(coinW, coinH),
                cornerRadius = CornerRadius(coinRadius)
            )

            // Highlights
            drawLine(
                color = Color.White.copy(alpha = 0.4f),
                start = Offset(x + 4.dp.toPx(), y + 2.dp.toPx()),
                end = Offset(x + coinW - 4.dp.toPx(), y + 2.dp.toPx()),
                strokeWidth = 1.dp.toPx()
            )
        }
    }
}

@Composable
fun IsometricGlobeChain(modifier: Modifier = Modifier) {
    val infiniteTransition = rememberInfiniteTransition(label = "globeChain")
    val rotation by infiniteTransition.animateFloat(
        initialValue = 0f, targetValue = 360f,
        animationSpec = infiniteRepeatable(tween(30000, easing = LinearEasing)),
        label = "rotation"
    )
    val pulse by infiniteTransition.animateFloat(
        initialValue = 0.9f, targetValue = 1.05f,
        animationSpec = infiniteRepeatable(tween(2500, easing = EaseInOutSine), RepeatMode.Reverse),
        label = "pulse"
    )

    Canvas(modifier = modifier) {
        val w = size.width
        val h = size.height
        val cx = w / 2f
        val cy = h / 2f
        val globeR = w * 0.3f * pulse

        drawCircle(
            brush = Brush.radialGradient(
                colors = listOf(
                    Purple.copy(alpha = 0.3f),
                    Purple.copy(alpha = 0.05f),
                    Color.Transparent
                ),
                center = Offset(cx, cy), radius = globeR
            ),
            radius = globeR, center = Offset(cx, cy)
        )

        drawCircle(
            color = Purple.copy(alpha = 0.5f),
            radius = globeR,
            center = Offset(cx, cy),
            style = Stroke(width = 1.5.dp.toPx())
        )

        drawOval(
            color = Purple.copy(alpha = 0.3f),
            topLeft = Offset(cx - globeR, cy - globeR * 0.5f),
            size = Size(globeR * 2, globeR),
            style = Stroke(width = 1.dp.toPx())
        )
        drawOval(
            color = Purple.copy(alpha = 0.3f),
            topLeft = Offset(cx - globeR * 0.5f, cy - globeR),
            size = Size(globeR, globeR * 2),
            style = Stroke(width = 1.dp.toPx())
        )

        val chainCount = 6
        val chainR = globeR * 0.12f
        for (i in 0 until chainCount) {
            val angle = Math.toRadians((rotation + i * (360.0 / chainCount)).toDouble())
            val dist = globeR * 0.85f
            val bx = cx + dist * cos(angle).toFloat()
            val by = cy + dist * sin(angle).toFloat() * 0.5f

            drawCircle(
                brush = Brush.radialGradient(
                    colors = listOf(Lime, Lime.copy(alpha = 0.3f)),
                    center = Offset(bx, by), radius = chainR
                ),
                radius = chainR, center = Offset(bx, by)
            )
            drawCircle(
                color = Color.White.copy(alpha = 0.4f),
                radius = chainR * 0.3f,
                center = Offset(bx - chainR * 0.2f, by - chainR * 0.2f)
            )

            val nextAngle = Math.toRadians((rotation + (i + 1) * (360.0 / chainCount)).toDouble())
            val nx = cx + dist * cos(nextAngle).toFloat()
            val ny = cy + dist * sin(nextAngle).toFloat() * 0.5f
            drawLine(
                color = Lime.copy(alpha = 0.4f),
                start = Offset(bx, by),
                end = Offset(nx, ny),
                strokeWidth = 1.dp.toPx()
            )
        }

        drawCircle(
            brush = Brush.radialGradient(
                colors = listOf(Lime.copy(alpha = 0.15f), Color.Transparent),
                center = Offset(cx, cy), radius = globeR * 1.5f
            ),
            radius = globeR * 1.5f, center = Offset(cx, cy)
        )
    }
}
