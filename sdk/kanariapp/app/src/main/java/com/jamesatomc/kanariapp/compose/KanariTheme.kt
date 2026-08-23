package com.jamesatomc.kanariapp.compose

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// Premium Dark Crypto Palette
private val BackgroundDark = Color(0xFF0B0E11)
private val PrimaryGreen = Color(0xFF00E676)
private val SurfaceDark = Color(0xFF1E2329)
private val OnSurfaceDark = Color(0xFFEAECEF)
private val OnBackgroundDark = Color(0xFFFFFFFF)

private val DarkColorScheme = darkColorScheme(
    primary = PrimaryGreen,
    onPrimary = Color(0xFF000000),
    background = BackgroundDark,
    onBackground = OnBackgroundDark,
    surface = SurfaceDark,
    onSurface = OnSurfaceDark,
    surfaceVariant = Color(0xFF2B3139),
    onSurfaceVariant = Color(0xFF929AA5),
    outline = Color(0xFF474D57)
)

private val LightColorScheme = lightColorScheme(
    primary = PrimaryGreen,
    onPrimary = Color.White,
    // Add light colors if needed, but the focus is Premium Dark
)

@Composable
fun KanariTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit
) {
    val colorScheme = if (darkTheme) DarkColorScheme else DarkColorScheme // Forcing dark for "Premium Dark" feel or could fallback

    MaterialTheme(
        colorScheme = colorScheme,
        content = content
    )
}
