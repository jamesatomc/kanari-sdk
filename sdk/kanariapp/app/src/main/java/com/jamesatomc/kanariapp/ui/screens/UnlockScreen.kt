package com.jamesatomc.kanariapp.ui.screens

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import com.jamesatomc.kanariapp.ui.components.PinVerificationContent
import com.jamesatomc.kanariapp.ui.components.ScaffoldWithBackBar
import com.jamesatomc.kanariapp.ui.components.findFragmentActivity
import com.jamesatomc.kanariapp.ui.components.rememberBiometricAvailable
import com.jamesatomc.kanariapp.ui.components.showBiometricPrompt
import com.jamesatomc.kanariapp.wallet.WalletViewModel

@Composable
fun UnlockScreen(viewModel: WalletViewModel, onUnlockSuccess: () -> Unit, onBack: () -> Unit) {
    val context = LocalContext.current
    val activity = context.findFragmentActivity()
    val canUseBiometric = rememberBiometricAvailable(viewModel)

    fun onBiometric() {
        if (activity == null) return
        showBiometricPrompt(
            activity = activity,
            title = "Unlock Kanari Wallet",
            subtitle = "Use biometrics to unlock",
            onSuccess = { if (viewModel.unlockWithBiometric()) onUnlockSuccess() }
        )
    }

    ScaffoldWithBackBar(title = "Unlock Wallet", onBack = onBack) { padding ->
        PinVerificationContent(
            title = "Welcome Back",
            subtitle = "Enter your 6-digit PIN to unlock your wallet",
            onVerify = { pin -> viewModel.unlock(pin) },
            onSuccess = { _: String -> onUnlockSuccess() },
            biometricEnabled = canUseBiometric,
            onBiometric = ::onBiometric,
            modifier = Modifier.fillMaxSize().padding(padding)
        )
    }
}
