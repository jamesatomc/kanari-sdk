package com.jamesatomc.kanariapp.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AlternateEmail
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.automirrored.filled.Login
import androidx.compose.material.icons.filled.Visibility
import androidx.compose.material.icons.filled.VisibilityOff
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
import com.jamesatomc.kanariapp.ui.components.AuthHeroSection
import com.jamesatomc.kanariapp.ui.components.ErrorBanner
import com.jamesatomc.kanariapp.ui.components.LoadingButton
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LoginScreen(
    onLoginSuccess: () -> Unit,
    onNavigateToRegister: () -> Unit,
    onNavigateToWalletGen: () -> Unit
) {
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var isLoading by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var passwordVisible by remember { mutableStateOf(false) }

    val scope = rememberCoroutineScope()
    val client = remember { KanariClient(KanariEnvironment.dev) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Login", style = MaterialTheme.typography.titleLarge) },
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
                title = "Welcome Back",
                subtitle = "Sign in to access your Kanari wallet and synced sessions."
            )

            Spacer(Modifier.height(16.dp))

            Surface(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(16.dp),
                color = MaterialTheme.colorScheme.surfaceContainerLowest,
                border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
            ) {
                Column(modifier = Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text("Account", style = MaterialTheme.typography.titleMedium)

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
                        label = { Text("Password") }, placeholder = { Text("Enter your password") },
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

                    if (error != null) ErrorBanner(error = error!!)
                }
            }

            Spacer(Modifier.height(24.dp))

            LoadingButton(
                onClick = {
                    scope.launch {
                        isLoading = true; error = null
                        val response = client.login(email, password)
                        if (response?.success == true) onLoginSuccess() else error = response?.error ?: "Login failed"
                        isLoading = false
                    }
                },
                text = "Login",
                icon = Icons.AutoMirrored.Filled.Login,
                enabled = email.isNotEmpty() && password.isNotEmpty(),
                isLoading = isLoading,
                modifier = Modifier.fillMaxWidth()
            )

            Spacer(Modifier.height(12.dp))

            TextButton(onClick = onNavigateToRegister, modifier = Modifier.fillMaxWidth().height(56.dp)) {
                Text("Don't have an account? Register")
            }

            Spacer(Modifier.height(8.dp))
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            Spacer(Modifier.height(8.dp))

            OutlinedButton(
                onClick = onNavigateToWalletGen,
                modifier = Modifier.fillMaxWidth().height(56.dp),
                shape = RoundedCornerShape(12.dp)
            ) {
                Text("Create or Import Local Wallet")
            }

            Spacer(Modifier.height(16.dp))
        }
    }
}
