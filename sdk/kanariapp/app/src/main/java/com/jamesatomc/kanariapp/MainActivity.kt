package com.jamesatomc.kanariapp

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import com.jamesatomc.kanariapp.wallet.WalletManagerScreen
import com.jamesatomc.kanariapp.compose.KanariTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        
        // เพิ่มบรรทัดนี้เพื่อขยายขีดจำกัดหน่วยความจำสำหรับ Native Library (PQ algorithms)
        System.setProperty("jna.nosys", "true")
        
        enableEdgeToEdge()
        setContent {
            KanariTheme {
                WalletManagerScreen()
            }
        }
    }
}
