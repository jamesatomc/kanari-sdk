package com.kanari.sample

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.kanari.kanari_crypto.compose.KanariTheme
import com.kanari.kanari_crypto.compose.KeyGenerationScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        
        // Add this line to raise the memory limit for the Native Library (PQ algorithms)
        System.setProperty("jna.nosys", "true")
        
        enableEdgeToEdge()
        setContent {
            KanariTheme {
                KeyGenerationScreen()
            }
        }
    }
}
