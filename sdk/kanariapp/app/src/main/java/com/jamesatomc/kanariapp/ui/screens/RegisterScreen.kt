package com.jamesatomc.kanariapp.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AlternateEmail
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.PersonAdd
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import com.jamesatomc.kanariapp.network.KanariClient
import com.jamesatomc.kanariapp.network.models.KanariEnvironment
import com.jamesatomc.kanariapp.network.models.RegisterRequest
import com.jamesatomc.kanariapp.ui.components.AuthHeroSection
import com.jamesatomc.kanariapp.ui.components.ErrorBanner
import com.jamesatomc.kanariapp.ui.components.LoadingButton
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RegisterScreen(
    onRegisterSuccess: () -> Unit,
    onNavigateToLogin: () -> Unit
) {
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var confirmPassword by remember { mutableStateOf("") }
    var isLoading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var passwordVisible by remember { mutableStateOf(false) }
    var confirmVisible by remember { mutableStateOf(false) }

    val scope = rememberCoroutineScope()
    val client = remember { KanariClient(KanariEnvironment.dev) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Register", style = MaterialTheme.typography.titleLarge) },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface)
            )
        },
        containerColor = MaterialTheme.colorScheme.surface
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp),
            horizontalAlignment = Alignment.CenterHorizontally
        ) {
            Spacer(Modifier.height(16.dp))

            AuthHeroSection(
                title = "Create Your Account",
                subtitle = "Set up your Kanari account and choose the wallet that fits your use case."
            )

            Spacer(Modifier.height(16.dp))

            Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surfaceContainerLowest,
                border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
            ) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text("Account Details", style = MaterialTheme.typography.titleMedium)

                    OutlinedTextField(
                        value = email,
                        onValueChange = { email = it },
                        label = { Text("Email") },
                        placeholder = { Text("user@example.com") },
                        leadingIcon = { Icon(Icons.Default.AlternateEmail, contentDescription = null) },
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(12.dp),
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
                        enabled = !isLoading
                    )

                    OutlinedTextField(
                        value = password, onValueChange = { password = it },
                        label = { Text("Password") }, placeholder = { Text("Strong password required") },
                        leadingIcon = { Icon(Icons.Default.Lock, contentDescription = null) },
                        trailingIcon = {
                            IconButton(onClick = { passwordVisible = !passwordVisible }) {
                                Icon(
                                    if (passwordVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                    contentDescription = null
                                )
                            }
                        },
                        visualTransformation = if (passwordVisible) VisualTransformation.None else PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp),
                        singleLine = true, enabled = !isLoading
                    )

                    Surface(
                        modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp),
                        color = MaterialTheme.colorScheme.surfaceContainerHighest,
                        border = androidx.compose.foundation.BorderStroke(
                            1.dp,
                            MaterialTheme.colorScheme.outline.copy(alpha = 0.2f)
                        )
                    ) {
                        Column(modifier = Modifier.padding(14.dp)) {
                            Text("Password requirements", style = MaterialTheme.typography.labelLarge)
                            Spacer(Modifier.height(8.dp))
                            RequirementRow("At least 8 characters", password.length >= 8)
                            RequirementRow("One uppercase letter", password.contains(Regex("[A-Z]")))
                            RequirementRow("One lowercase letter", password.contains(Regex("[a-z]")))
                            RequirementRow("One digit", password.contains(Regex("[0-9]")))
                            RequirementRow(
                                "One special character",
                                password.contains(Regex("[!@#$%^&*(),.?\":{}|<>]"))
                            )
                        }
                    }

                    OutlinedTextField(
                        value = confirmPassword, onValueChange = { confirmPassword = it },
                        label = { Text("Confirm Password") }, placeholder = { Text("Re-enter your password") },
                        leadingIcon = { Icon(Icons.Default.Lock, contentDescription = null) },
                        trailingIcon = {
                            IconButton(onClick = { confirmVisible = !confirmVisible }) {
                                Icon(
                                    if (confirmVisible) Icons.Default.VisibilityOff else Icons.Default.Visibility,
                                    contentDescription = null
                                )
                            }
                        },
                        visualTransformation = if (confirmVisible) VisualTransformation.None else PasswordVisualTransformation(),
                        modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(12.dp),
                        singleLine = true, enabled = !isLoading
                    )
                }
            }

            Spacer(Modifier.height(16.dp))

            Surface(
                modifier = Modifier.fillMaxWidth(), shape = RoundedCornerShape(18.dp),
                color = MaterialTheme.colorScheme.surfaceContainerLowest,
                border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
            ) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text("Wallet Cryptography", style = MaterialTheme.typography.titleMedium)
                    Text(
                        "Ed25519 — Fast and secure default",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }

            Spacer(Modifier.height(16.dp))

            if (error != null) {
                ErrorBanner(error = error!!); Spacer(Modifier.height(16.dp))
            }

            LoadingButton(
                onClick = {
                    scope.launch {
                        isLoading = true; error = null
                        try {
                            val response = client.authService.register(RegisterRequest(email, password))
                            if (response.isSuccessful && response.body()?.success == true) onRegisterSuccess()
                            else error = response.body()?.error ?: "Registration failed"
                        } catch (e: Exception) {
                            error = e.message
                        }
                        isLoading = false
                    }
                },
                text = "Create Account",
                icon = Icons.Default.PersonAdd,
                enabled = email.isNotEmpty() && password.isNotEmpty() && confirmPassword == password,
                isLoading = isLoading,
                modifier = Modifier.fillMaxWidth()
            )

            Spacer(Modifier.height(12.dp))

            TextButton(onClick = onNavigateToLogin, modifier = Modifier.fillMaxWidth().height(56.dp)) {
                Text("Already have an account? Login")
            }

            Spacer(Modifier.height(16.dp))
        }
    }
}

@Composable
private fun RequirementRow(text: String, isValid: Boolean) {
    Row(
        modifier = Modifier.padding(vertical = 3.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Icon(
            if (isValid) Icons.Default.CheckCircle else Icons.Default.Cancel,
            contentDescription = null, modifier = Modifier.size(16.dp),
            tint = if (isValid) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.onSurfaceVariant
        )
        Text(
            text, style = MaterialTheme.typography.bodySmall,
            color = if (isValid) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}
