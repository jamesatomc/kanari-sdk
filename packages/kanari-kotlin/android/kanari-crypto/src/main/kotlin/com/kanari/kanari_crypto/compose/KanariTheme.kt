package com.kanari.kanari_crypto.compose

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val KanariGreen = Color(0xFF00C853)
private val KanariGreenDark = Color(0xFF00A344)
private val KanariSurfaceDark = Color(0xFF121212)
private val KanariSurfaceLight = Color(0xFFFAFAFA)

private val LightColors = lightColorScheme(
    primary = KanariGreen,
    onPrimary = Color.White,
    secondary = KanariGreenDark,
    surface = KanariSurfaceLight,
)

private val DarkColors = darkColorScheme(
    primary = KanariGreen,
    onPrimary = Color.Black,
    secondary = KanariGreenDark,
    surface = KanariSurfaceDark,
)

@Composable
fun KanariTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit,
) {
    MaterialTheme(
        colorScheme = if (darkTheme) DarkColors else LightColors,
        content = content,
    )
}
