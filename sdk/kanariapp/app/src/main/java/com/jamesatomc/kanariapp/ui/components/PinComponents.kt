package com.jamesatomc.kanariapp.ui.components

import android.annotation.SuppressLint
import android.widget.Toast
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.ExperimentalAnimationApi
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Backspace
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class PinState {
    var entered by mutableStateOf("")
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var isChecking by mutableStateOf(false)
        internal set

    fun onNumber(num: String) {
        if (isChecking || entered.length >= 6) return
        entered += num
        error = null
    }

    fun onBackspace() {
        if (entered.isNotEmpty() && !isChecking) {
            entered = entered.dropLast(1)
            error = null
        }
    }

    fun reset() {
        entered = ""
        error = null
        isChecking = false
    }

    fun fail(msg: String) {
        error = msg
        entered = ""
        isChecking = false
    }
}

@Composable
fun rememberPinState(): PinState = remember { PinState() }

@Composable
fun PinCircles(
    length: Int,
    totalLength: Int = 6,
    @SuppressLint("ModifierParameter") modifier: Modifier = Modifier
) {
    val primary = MaterialTheme.colorScheme.primary
    val outline = MaterialTheme.colorScheme.outlineVariant
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically
    ) {
        repeat(totalLength) { index ->
            val filled = index < length
            Surface(
                shape = CircleShape,
                color = if (filled) primary else androidx.compose.ui.graphics.Color.Transparent,
                border = BorderStroke(2.dp, if (filled) primary else outline),
                modifier = Modifier.size(18.dp)
            ) {}
        }
    }
}

@Composable
fun PinNumberPad(
    onNumberPressed: (String) -> Unit,
    onBackspacePressed: () -> Unit,
    biometricEnabled: Boolean = false,
    onBiometricPressed: (() -> Unit)? = null,
    @SuppressLint("ModifierParameter") modifier: Modifier = Modifier
) {
    val numbers = listOf(
        listOf("1", "2", "3"),
        listOf("4", "5", "6"),
        listOf("7", "8", "9")
    )
    Column(
        modifier = modifier
            .widthIn(max = 280.dp)
            .padding(horizontal = 12.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        numbers.forEach { row ->
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterHorizontally)
            ) {
                row.forEach { num ->
                    NumberButton(number = num, onClick = onNumberPressed)
                }
            }
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically
        ) {
            if (biometricEnabled) IconButton(
                onClick = { onBiometricPressed?.invoke() },
                modifier = Modifier.size(56.dp)
            ) {
                Icon(
                    Icons.Default.Fingerprint,
                    contentDescription = "Biometric",
                    modifier = Modifier.size(26.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            else Spacer(Modifier.size(56.dp))
            NumberButton(number = "0", onClick = onNumberPressed)
            IconButton(onClick = onBackspacePressed, modifier = Modifier.size(56.dp)) {
                Icon(
                    Icons.AutoMirrored.Filled.Backspace,
                    contentDescription = "Backspace",
                    modifier = Modifier.size(26.dp),
                    tint = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }
    }
}

@Composable
private fun NumberButton(number: String, onClick: (String) -> Unit) {
    Surface(
        shape = CircleShape,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
        modifier = Modifier.size(56.dp),
        onClick = { onClick(number) }
    ) {
        Box(contentAlignment = Alignment.Center, modifier = Modifier.fillMaxSize()) {
            Text(
                number,
                style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.SemiBold),
                color = MaterialTheme.colorScheme.onSurface
            )
        }
    }
}

@OptIn(ExperimentalAnimationApi::class)
@Composable
fun PinEntryHeader(
    title: String,
    subtitle: String,
    enteredLength: Int,
    totalLength: Int = 6,
    errorText: String? = null,
    isChecking: Boolean = false
) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.fillMaxWidth()) {
        Text(
            title,
            style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold),
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.Center
        )
        Spacer(Modifier.height(12.dp))
        Text(
            subtitle,
            style = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurfaceVariant),
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(horizontal = 32.dp)
        )
        Spacer(Modifier.height(24.dp))
        PinCircles(length = enteredLength, totalLength = totalLength)
        AnimatedContent(targetState = errorText, label = "error") { err ->
            if (err == null) Spacer(Modifier.height(20.dp))
            else Text(
                err,
                style = MaterialTheme.typography.bodyMedium.copy(
                    color = MaterialTheme.colorScheme.error,
                    fontWeight = FontWeight.SemiBold
                ),
                modifier = Modifier.padding(top = 8.dp)
            )
        }
        if (isChecking) {
            CircularProgressIndicator(
                modifier = Modifier.padding(top = 8.dp).size(22.dp),
                strokeWidth = 2.dp,
                color = MaterialTheme.colorScheme.primary
            )
        } else Spacer(Modifier.height(24.dp))
    }
}

@Composable
fun PinVerificationContent(
    title: String,
    subtitle: String,
    onVerify: (String) -> Boolean = { false },
    onVerifyAsync: (suspend (String) -> Boolean)? = null,
    onSuccess: (String) -> Unit,
    biometricEnabled: Boolean = false,
    onBiometric: (() -> Unit)? = null,
    @SuppressLint("ModifierParameter") modifier: Modifier = Modifier
) {
    val pin = rememberPinState()
    val scope = rememberCoroutineScope()
    val onNumberPressedStable = remember {
        { num: String ->
            pin.onNumber(num)
            if (pin.entered.length == 6) {
                pin.isChecking = true
                scope.launch {
                    val currentPin = pin.entered
                    val ok = try {
                        if (onVerifyAsync != null) onVerifyAsync(currentPin) else onVerify(currentPin)
                    } catch (_: Exception) {
                        false
                    }
                    if (ok) {
                        onSuccess(currentPin)
                    } else {
                        pin.fail("Invalid PIN")
                    }
                }
            }
        }
    }

    val onBackspacePressedStable = remember { { pin.onBackspace() } }

    Column(
        modifier = modifier.fillMaxSize().padding(horizontal = 24.dp, vertical = 24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        PinEntryHeader(
            title = title,
            subtitle = subtitle,
            enteredLength = pin.entered.length,
            errorText = pin.error,
            isChecking = pin.isChecking
        )
        Spacer(Modifier.height(32.dp))
        PinNumberPad(
            onNumberPressed = onNumberPressedStable,
            onBackspacePressed = onBackspacePressedStable,
            biometricEnabled = biometricEnabled,
            onBiometricPressed = onBiometric
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChangePinFullScreenContent(
    onDismiss: () -> Unit,
    onConfirm: (String, String) -> Boolean = { _, _ -> false },
    onConfirmAsync: (suspend (String, String) -> Boolean)? = null,
    biometricEnabled: Boolean = false,
    onBiometric: (() -> Unit)? = null
) {
    val pin = rememberPinState()
    var step by remember { mutableIntStateOf(0) }
    var currentPin by remember { mutableStateOf("") }
    var newPin by remember { mutableStateOf("") }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val title = when (step) {
        0 -> "Current PIN"; 1 -> "New PIN"; else -> "Confirm New PIN"
    }
    val subtitle = when (step) {
        0 -> "Enter current 6-digit PIN"; 1 -> "Enter new 6-digit PIN"; else -> "Re-enter new PIN to confirm"
    }
    val showBiometric = biometricEnabled && step == 0

    Scaffold(topBar = {
        TopAppBar(
            title = { Text("Change PIN") },
            navigationIcon = {
                IconButton(onClick = {
                    if (step > 0) {
                        step--; pin.reset()
                    } else onDismiss()
                }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
            })
    }) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 24.dp, vertical = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            PinEntryHeader(
                title = title,
                subtitle = subtitle,
                enteredLength = pin.entered.length,
                errorText = pin.error,
                isChecking = pin.isChecking
            )
            Spacer(Modifier.height(32.dp))
            val onNumberPressedStable = remember(step, currentPin, newPin) {
                { num: String ->
                    pin.onNumber(num)
                    if (pin.entered.length == 6) {
                        when (step) {
                            0 -> {
                                currentPin = pin.entered; pin.reset(); step = 1
                            }

                            1 -> {
                                newPin = pin.entered; pin.reset(); step = 2
                            }

                            2 -> {
                                if (pin.entered != newPin) pin.fail("PINs do not match")
                                else if (currentPin == newPin) pin.fail("New PIN must be different")
                                else {
                                    pin.isChecking = true
                                    scope.launch {
                                        val ok = try {
                                            if (onConfirmAsync != null) onConfirmAsync(currentPin, pin.entered)
                                            else withContext(Dispatchers.Default) { onConfirm(currentPin, pin.entered) }
                                        } catch (_: Exception) {
                                            false
                                        }
                                        if (ok) {
                                            Toast.makeText(context, "PIN changed successfully", Toast.LENGTH_SHORT)
                                                .show()
                                            onDismiss()
                                        } else pin.fail("Incorrect current PIN or invalid new PIN")
                                    }
                                }
                            }
                        }
                    }
                }
            }
            val onBackspacePressedStable = remember { { pin.onBackspace() } }

            PinNumberPad(
                onNumberPressed = onNumberPressedStable,
                onBackspacePressed = onBackspacePressedStable,
                biometricEnabled = showBiometric,
                onBiometricPressed = onBiometric
            )
        }
    }
}
