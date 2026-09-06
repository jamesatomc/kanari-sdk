package com.jamesatomc.kanariapp.ui.screens

import android.widget.Toast
import androidx.biometric.BiometricManager
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Logout
import androidx.compose.material.icons.filled.BrightnessAuto
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.DarkMode
import androidx.compose.material.icons.filled.Fingerprint
import androidx.compose.material.icons.filled.LightMode
import androidx.compose.material.icons.filled.Public
import androidx.compose.material.icons.filled.Security
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.jamesatomc.kanariapp.network.models.KanariEnvironment
import com.jamesatomc.kanariapp.ui.components.ChangePinFullScreenContent
import com.jamesatomc.kanariapp.ui.components.LoadingButton
import com.jamesatomc.kanariapp.ui.components.ScaffoldWithBackBar
import com.jamesatomc.kanariapp.ui.components.showBiometricPrompt
import com.jamesatomc.kanariapp.ui.theme.ThemeMode
import com.jamesatomc.kanariapp.wallet.WalletViewModel
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(viewModel: WalletViewModel, onLogout: () -> Unit, onBack: () -> Unit) {
    val currentEnv by viewModel.environment.collectAsState()
    val themeMode by viewModel.themeMode.collectAsState()
    val scope = rememberCoroutineScope()
    var showPinDialog by remember { mutableStateOf(false) }
    var showEnvDialog by remember { mutableStateOf(false) }
    var showThemeDialog by remember { mutableStateOf(false) }
    ScaffoldWithBackBar(
        title = "Settings",
        onBack = onBack,
        containerColor = MaterialTheme.colorScheme.background
    ) { padding ->
        Column(Modifier.fillMaxSize().padding(padding).padding(16.dp)) {
            Text(
                "APPEARANCE",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(8.dp))
            SettingsItem(
                title = "Theme", subtitle = when (themeMode) {
                    ThemeMode.LIGHT -> "Light"; ThemeMode.DARK -> "Dark"; ThemeMode.SYSTEM -> "System"
                }, icon = when (themeMode) {
                    ThemeMode.LIGHT -> Icons.Default.LightMode; ThemeMode.DARK -> Icons.Default.DarkMode; ThemeMode.SYSTEM -> Icons.Default.BrightnessAuto
                }, onClick = { showThemeDialog = true })
            Spacer(Modifier.height(24.dp))
            Text(
                "SECURITY",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(8.dp))
            SettingsItem(
                title = "Change PIN",
                subtitle = "Update your 6-digit PIN",
                icon = Icons.Default.Security,
                onClick = { showPinDialog = true })
            val context = LocalContext.current
            val activity = context as? androidx.fragment.app.FragmentActivity
            val biometricManager = remember { BiometricManager.from(context) }
            val biometricStatus = remember {
                try {
                    biometricManager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                } catch (_: Exception) {
                    BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE
                }
            }
            val biometricAvailable = biometricStatus == BiometricManager.BIOMETRIC_SUCCESS
            val biometricEnabled by viewModel.biometricEnabled.collectAsState()
            val biometricSubtitle = when {
                biometricEnabled -> "Enabled - use fingerprint to unlock"
                biometricStatus == BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "No fingerprint enrolled - add in system settings"
                biometricStatus == BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE -> "No biometric hardware"
                biometricStatus == BiometricManager.BIOMETRIC_ERROR_HW_UNAVAILABLE -> "Biometric unavailable"
                else -> "Tap to enable fingerprint / face unlock"
            }
            SettingsSwitchItem(
                title = "Biometric Unlock",
                subtitle = biometricSubtitle,
                icon = Icons.Default.Fingerprint,
                checked = biometricEnabled,
                enabled = biometricAvailable,
                onCheckedChange = { enabled ->
                    scope.launch {
                        if (!enabled) {
                            viewModel.setBiometricEnabled(false)
                            Toast.makeText(context, "Biometric disabled", Toast.LENGTH_SHORT).show()
                        } else {
                            if (activity == null) {
                                val ok = viewModel.setBiometricEnabled(true)
                                Toast.makeText(
                                    context,
                                    if (ok) "Biometric enabled" else "Unlock with PIN first",
                                    Toast.LENGTH_SHORT
                                ).show()
                                return@launch
                            }
                            showBiometricPrompt(
                                activity = activity,
                                title = "Enable Biometric Unlock",
                                subtitle = "Authenticate to enable fingerprint unlock",
                                onSuccess = {
                                    scope.launch {
                                        val ok = viewModel.setBiometricEnabled(true)
                                        Toast.makeText(
                                            context,
                                            if (ok) "Biometric enabled" else "Unlock with PIN first",
                                            Toast.LENGTH_SHORT
                                        ).show()
                                    }
                                },
                                onError = { errorCode, errString ->
                                    if (errorCode != androidx.biometric.BiometricPrompt.ERROR_USER_CANCELED && errorCode != androidx.biometric.BiometricPrompt.ERROR_NEGATIVE_BUTTON) {
                                        Toast.makeText(context, "Biometric error: $errString", Toast.LENGTH_SHORT)
                                            .show()
                                    }
                                },
                                onFailed = {
                                    Toast.makeText(context, "Biometric not recognized", Toast.LENGTH_SHORT).show()
                                }
                            )
                        }
                    }
                })
            Spacer(Modifier.height(24.dp))
            Text(
                "NETWORK",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Spacer(Modifier.height(8.dp))
            SettingsItem(
                title = "Environment",
                subtitle = currentEnv.name,
                icon = Icons.Default.Public,
                onClick = { showEnvDialog = true })
            Spacer(Modifier.weight(1f))
            LoadingButton(
                onClick = onLogout,
                text = "Logout",
                icon = Icons.AutoMirrored.Filled.Logout,
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error)
            )
        }
    }
    if (showPinDialog) ChangePinDialog(
        onDismiss = { showPinDialog = false },
        onConfirmAsync = { old, new -> viewModel.changePin(old, new) })
    if (showEnvDialog) EnvironmentDialog(
        currentEnv = currentEnv,
        onDismiss = { showEnvDialog = false },
        onSelect = { viewModel.setEnvironment(it); showEnvDialog = false })
    if (showThemeDialog) ThemeDialog(
        current = themeMode,
        onDismiss = { showThemeDialog = false },
        onSelect = { viewModel.setThemeMode(it); showThemeDialog = false })
}

@Composable
fun ChangePinDialog(onDismiss: () -> Unit, onConfirm: (String, String) -> Boolean = { _, _ -> false }, onConfirmAsync: (suspend (String, String) -> Boolean)? = null) {
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(
            usePlatformDefaultWidth = false,
            decorFitsSystemWindows = false
        )
    ) {
        Surface(color = MaterialTheme.colorScheme.background) {
            ChangePinFullScreenContent(onDismiss = onDismiss, onConfirm = onConfirm, onConfirmAsync = onConfirmAsync)
        }
    }
}

@Composable
fun EnvironmentDialog(currentEnv: KanariEnvironment, onDismiss: () -> Unit, onSelect: (KanariEnvironment) -> Unit) {
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Select Environment") }, text = {
        Column {
            KanariEnvironment.entries.forEach { env ->
                Row(
                    Modifier.fillMaxWidth().clickable { onSelect(env) }.padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    RadioButton(selected = env == currentEnv, onClick = { onSelect(env) })
                    Spacer(Modifier.width(8.dp))
                    Text(env.name)
                }
            }
        }
    }, confirmButton = {
        TextButton(onClick = onDismiss) { Text("Done") }
    })
}

@Composable
fun ThemeDialog(current: ThemeMode, onDismiss: () -> Unit, onSelect: (ThemeMode) -> Unit) {
    AlertDialog(onDismissRequest = onDismiss, title = { Text("Select Theme") }, text = {
        Column {
            ThemeMode.entries.forEach { mode ->
                val title = when (mode) {
                    ThemeMode.LIGHT -> "Light"; ThemeMode.DARK -> "Dark"; ThemeMode.SYSTEM -> "System"
                }
                val icon = when (mode) {
                    ThemeMode.LIGHT -> Icons.Default.LightMode; ThemeMode.DARK -> Icons.Default.DarkMode; ThemeMode.SYSTEM -> Icons.Default.BrightnessAuto
                }
                Row(
                    Modifier.fillMaxWidth().clickable { onSelect(mode) }.padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    Spacer(Modifier.width(16.dp))
                    Text(title, modifier = Modifier.weight(1f))
                    RadioButton(selected = mode == current, onClick = { onSelect(mode) })
                }
            }
        }
    }, confirmButton = {
        TextButton(onClick = onDismiss) { Text("Done") }
    })
}

@Composable
fun SettingsItem(title: String, subtitle: String? = null, icon: ImageVector, onClick: () -> Unit) {
    Surface(onClick = onClick, color = Color.Transparent) {
        Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(icon, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.width(16.dp))
            Column(Modifier.weight(1f)) {
                Text(title, fontWeight = FontWeight.Bold)
                if (subtitle != null) Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            Icon(
                Icons.Default.ChevronRight,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
fun SettingsSwitchItem(
    title: String,
    subtitle: String? = null,
    icon: ImageVector,
    checked: Boolean,
    enabled: Boolean = true,
    onCheckedChange: (Boolean) -> Unit
) {
    Surface(color = Color.Transparent) {
        Row(Modifier.fillMaxWidth().padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(
                icon,
                contentDescription = null,
                tint = if (enabled) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurfaceVariant.copy(
                    alpha = 0.4f
                )
            )
            Spacer(Modifier.width(16.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    title,
                    fontWeight = FontWeight.Bold,
                    color = if (enabled) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurface.copy(
                        alpha = 0.4f
                    )
                )
                if (subtitle != null) Text(
                    subtitle,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = if (enabled) 1f else 0.6f)
                )
            }
            Switch(checked = checked, onCheckedChange = onCheckedChange, enabled = enabled)
        }
    }
}
