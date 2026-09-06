@file:OptIn(ExperimentalMaterial3Api::class, androidx.compose.animation.ExperimentalAnimationApi::class)

package com.jamesatomc.kanariapp.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.animation.core.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.jamesatomc.kanariapp.ui.components.*
import com.jamesatomc.kanariapp.ui.theme.KanariColors
import com.jamesatomc.kanariapp.ui.theme.LocalKanariGradients
import com.jamesatomc.kanariapp.wallet.WalletRecord
import com.jamesatomc.kanariapp.wallet.WalletViewModel

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DashboardScreen(
    viewModel: WalletViewModel,
    onNavigateToReceive: () -> Unit,
    onNavigateToSettings: () -> Unit,
    onNavigateToSend: () -> Unit,
    onNavigateToWalletGen: () -> Unit,
    modifier: Modifier = Modifier
) {
    val accountInfo by viewModel.accountInfo.collectAsStateWithLifecycle()
    val tokenBalances by viewModel.tokenBalances.collectAsStateWithLifecycle()
    val wallets by viewModel.wallets.collectAsStateWithLifecycle()
    val activeWallet by viewModel.activeWallet.collectAsStateWithLifecycle()
    val isLoading by viewModel.isLoading.collectAsStateWithLifecycle()
    val environment by viewModel.environment.collectAsStateWithLifecycle()
    val error by viewModel.error.collectAsStateWithLifecycle()
    val pagerState = rememberPagerState(pageCount = { wallets.size + 1 })
    var showEnvDialog by remember { mutableStateOf(false) }
    var walletToDelete by remember { mutableStateOf<WalletRecord?>(null) }
    var walletForDetails by remember { mutableStateOf<WalletRecord?>(null) }
    LaunchedEffect(activeWallet) {
        val idx =
            wallets.indexOfFirst { it.id == activeWallet?.id }; if (idx >= 0 && pagerState.currentPage != idx) pagerState.animateScrollToPage(
        idx
    )
    }
    LaunchedEffect(pagerState.currentPage) { if (pagerState.currentPage < wallets.size) viewModel.switchWallet(wallets[pagerState.currentPage]) }
    Scaffold(
        modifier = modifier,
        topBar = {
            KanariTopBar(
                environmentName = environment.name,
                onEnvClick = { showEnvDialog = true },
                onReceive = onNavigateToReceive,
                onSettings = onNavigateToSettings
            )
        },
        containerColor = MaterialTheme.colorScheme.background
    ) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            HorizontalPager(
                state = pagerState,
                modifier = Modifier.fillMaxWidth().height(210.dp),
                contentPadding = PaddingValues(horizontal = 16.dp),
                pageSpacing = 12.dp
            ) { page ->
                if (page < wallets.size) {
                    val w = wallets[page]
                    Box(
                        modifier = Modifier.graphicsLayer {
                            val pageOffset = (pagerState.currentPage - page) + pagerState.currentPageOffsetFraction
                            val scale = androidx.compose.ui.util.lerp(
                                0.88f,
                                1f,
                                1f - kotlin.math.abs(pageOffset).coerceIn(0f, 1f)
                            )
                            scaleX = scale; scaleY = scale
                            alpha = androidx.compose.ui.util.lerp(
                                0.6f,
                                1f,
                                1f - kotlin.math.abs(pageOffset).coerceIn(0f, 1f)
                            )
                        }
                    ) {
                        WalletCard(
                            w,
                            if (activeWallet?.id == w.id) tokenBalances else emptyList(),
                            onViewDetails = { walletForDetails = w })
                    }
                } else AddWalletCard(onNavigateToWalletGen)
            }
            DotsIndicator(
                totalDots = wallets.size + 1,
                selectedIndex = pagerState.currentPage,
                modifier = Modifier.fillMaxWidth()
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                LoadingButton(
                    onClick = onNavigateToSend,
                    text = "Send",
                    icon = Icons.AutoMirrored.Filled.Send,
                    modifier = Modifier.weight(1f),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = KanariColors.Lime,
                        contentColor = KanariColors.Ink
                    )
                )
                OutlinedButton(
                    onClick = onNavigateToReceive,
                    modifier = Modifier.weight(1f).height(60.dp),
                    shape = RoundedCornerShape(16.dp),
                    border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
                ) {
                    Icon(Icons.Default.QrCode, contentDescription = null, modifier = Modifier.size(20.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("Receive", style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.Bold))
                }
            }
            SectionHeader(title = "Assets")
            error?.let { ErrorBanner(error = it) }
            when {
                isLoading && tokenBalances.isEmpty() -> Box(
                    Modifier.fillMaxWidth().height(100.dp),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator()
                }

                wallets.isEmpty() -> Box(Modifier.fillMaxWidth().height(100.dp), contentAlignment = Alignment.Center) {
                    Text("No wallets found", color = MaterialTheme.colorScheme.onSurfaceVariant)
                }

                else -> tokenBalances.forEach { AssetItem(it) }
            }
            val objects = accountInfo?.ownedObjects ?: emptyList()
            if (objects.isNotEmpty()) {
                SectionHeader(title = "Objects & NFTs", modifier = Modifier.padding(top = 8.dp))
                objects.forEach { ObjectItem(it) }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
    if (showEnvDialog) EnvironmentDialog(
        currentEnv = environment,
        onDismiss = { showEnvDialog = false },
        onSelect = { viewModel.setEnvironment(it); showEnvDialog = false })
    walletToDelete?.let { w ->
        AlertDialog(
            onDismissRequest = { walletToDelete = null },
            title = { Text("Delete Wallet") },
            text = { Text("Are you sure you want to delete '${w.name}'?") },
            confirmButton = {
                TextButton(
                    onClick = { viewModel.deleteWallet(w); walletToDelete = null },
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error)
                ) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { walletToDelete = null }) { Text("Cancel") } })
    }
    walletForDetails?.let { w ->
        FullScreenDialog(onDismiss = { walletForDetails = null }) {
            WalletDetailFullScreen(
                wallet = w,
                viewModel = viewModel,
                onDismiss = { walletForDetails = null },
                onDeleteRequest = {
                    walletToDelete = w
                    walletForDetails = null
                }
            )
        }
    }
}

@Composable
fun WalletCard(
    wallet: WalletRecord,
    tokenBalances: List<com.jamesatomc.kanariapp.network.models.TokenBalance>,
    onViewDetails: () -> Unit
) {
    val curveInfo = remember(wallet.curveType) { getCurveInfo(wallet.curveType) }
    val gradients = LocalKanariGradients.current
    val infiniteTransition = rememberInfiniteTransition(label = "holographic")
    val holoOffset by infiniteTransition.animateFloat(
        initialValue = -1f,
        targetValue = 2f,
        animationSpec = infiniteRepeatable(tween(4000, easing = LinearEasing)),
        label = "holoOffset"
    )

    Card(
        Modifier.fillMaxSize(),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(24.dp),
        colors = CardDefaults.cardColors(containerColor = Color.Transparent),
        elevation = CardDefaults.cardElevation(4.dp)
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .background(
                    if (curveInfo.isPostQuantum) gradients.multi else gradients.secondary
                )
        ) {
            // Holographic sweep for PQ wallets
            if (curveInfo.isPostQuantum) {
                Box(
                    Modifier
                        .fillMaxSize()
                        .background(
                            Brush.linearGradient(
                                0.0f to Color.Transparent,
                                0.5f to Color.White.copy(alpha = 0.15f),
                                1.0f to Color.Transparent,
                                start = Offset(100f * holoOffset, 0f),
                                end = Offset(100f * holoOffset + 200f, 400f)
                            )
                        )
                )
            }

            IsometricCoinStack(
                modifier = Modifier
                    .size(130.dp)
                    .align(Alignment.BottomEnd)
                    .offset(x = 10.dp, y = 10.dp)
                    .graphicsLayer { alpha = 0.2f }
            )

            Column(Modifier.fillMaxSize().padding(24.dp)) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            Icons.Default.AccountBalanceWallet,
                            contentDescription = null,
                            tint = Color.White.copy(alpha = 0.8f),
                            modifier = Modifier.size(16.dp)
                        )
                        Spacer(Modifier.width(8.dp))
                        Text(
                            wallet.name,
                            style = MaterialTheme.typography.labelLarge.copy(
                                fontWeight = FontWeight.Bold,
                                shadow = Shadow(
                                    color = Color.Black.copy(alpha = 0.3f),
                                    offset = Offset(0f, 1f),
                                    blurRadius = 2f
                                )
                            ),
                            color = Color.White
                        )
                    }
                    IconButton(
                        onClick = onViewDetails,
                        modifier = Modifier.size(36.dp)
                    ) {
                        Icon(
                            Icons.Default.Visibility,
                            contentDescription = "View details",
                            modifier = Modifier.size(22.dp),
                            tint = Color.White
                        )
                    }
                }

                Spacer(Modifier.height(10.dp))
                CurveBadge(curveInfo)

                Spacer(Modifier.height(20.dp))
                val t = tokenBalances.find { it.tokenType == "0x2::kanari::KANARI" }
                Text(
                    "Balance",
                    style = MaterialTheme.typography.labelLarge.copy(
                        fontWeight = FontWeight.Bold,
                        shadow = Shadow(
                            color = Color.Black.copy(alpha = 0.3f),
                            offset = Offset(0f, 2f),
                            blurRadius = 4f
                        )
                    ),
                    color = Color.White.copy(alpha = 0.9f)
                )
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        formatAmount(t?.getEffectiveAmount() ?: 0L, t?.decimals ?: 9, 2),
                        style = MaterialTheme.typography.headlineMedium.copy(
                            fontWeight = FontWeight.Bold,
                            color = Color.White,
                            letterSpacing = 0.5.sp,
                            shadow = Shadow(
                                color = Color.Black.copy(alpha = 0.4f),
                                offset = Offset(0f, 4f),
                                blurRadius = 8f
                            )
                        )
                    )
                    Text(
                        " KANARI",
                        style = MaterialTheme.typography.titleSmall.copy(
                            fontWeight = FontWeight.Bold,
                            color = Color.White.copy(alpha = 0.75f),
                            shadow = Shadow(
                                color = Color.Black.copy(alpha = 0.3f),
                                offset = Offset(0f, 2f),
                                blurRadius = 4f
                            )
                        ),
                        modifier = Modifier.padding(bottom = 4.dp, start = 4.dp)
                    )
                }

                Spacer(Modifier.weight(1f))

                CopyableAddressRow(
                    wallet.address,
                    short = false,
                    textStyle = MaterialTheme.typography.labelMedium.copy(
                        color = Color.White.copy(alpha = 0.9f),
                        fontWeight = FontWeight.Bold,
                        shadow = Shadow(
                            color = Color.Black.copy(alpha = 0.3f),
                            offset = Offset(0f, 1f),
                            blurRadius = 2f
                        )
                    )
                )
            }
        }
    }
}

@Composable
fun AddWalletCard(onClick: () -> Unit) {
    Card(
        onClick = onClick,
        Modifier.fillMaxSize(),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
        border = androidx.compose.foundation.BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
    ) {
        Column(
            Modifier.fillMaxSize(),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.Center
        ) {
            FilledTonalIconButton(onClick = onClick) { Icon(Icons.Default.Add, contentDescription = null) }
            Spacer(Modifier.height(8.dp)); Text("Add Wallet", style = MaterialTheme.typography.titleSmall)
        }
    }
}

@Composable
fun AssetItem(token: com.jamesatomc.kanariapp.network.models.TokenBalance) {
    val displayType = remember(token.tokenType) {
        val parts = token.tokenType.split("::")
        if (parts.size >= 3) "${parts[parts.size - 2]}::${parts.last()}" else token.tokenType
    }
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
        shape = androidx.compose.foundation.shape.RoundedCornerShape(16.dp),
        modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp)
    ) {
        ListItem(
            leadingContent = { TokenIcon(token) },
            headlineContent = {
                Text(
                    token.symbol,
                    style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold)
                )
            },
            supportingContent = {
                Text(
                    displayType,
                    maxLines = 1,
                    style = MaterialTheme.typography.bodySmall.copy(
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f)
                    )
                )
            },
            trailingContent = {
                Text(
                    formatAmount(token.getEffectiveAmount(), token.decimals),
                    style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.ExtraBold),
                    color = MaterialTheme.colorScheme.onSurface
                )
            },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent)
        )
    }
}

@Composable
fun ObjectItem(obj: com.jamesatomc.kanariapp.network.models.ObjectInfo) {
    ListItem(
        headlineContent = { Text(obj.getEffectiveType().split("::").last()) },
        supportingContent = {
            Text(
                obj.id,
                maxLines = 1,
                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis
            )
        },
        leadingContent = { Icon(Icons.Default.Public, contentDescription = null) })
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WalletDetailFullScreen(
    wallet: WalletRecord,
    viewModel: WalletViewModel,
    onDismiss: () -> Unit,
    onDeleteRequest: () -> Unit
) {
    val ctx = LocalContext.current
    val activity = ctx.findFragmentActivity()
    var isVerified by remember { mutableStateOf(false) }
    var revealedKey by remember { mutableStateOf<String?>(null) }
    var revealedSeed by remember { mutableStateOf<String?>(null) }
    var keyVisible by remember { mutableStateOf(false) }
    var seedVisible by remember { mutableStateOf(false) }
    val hasSeed = wallet.mnemonicEncrypted != null
    val curveInfo = remember(wallet.curveType) { getCurveInfo(wallet.curveType) }
    val canUseBiometric = rememberBiometricAvailable(viewModel)

    fun onBiometric() {
        if (activity == null) return
        showBiometricPrompt(
            activity = activity,
            title = "Reveal Wallet Secrets",
            subtitle = "Use biometrics to unlock",
            onSuccess = {
                val k = viewModel.revealPrivateKeyWithBiometric(wallet)
                if (k != null) {
                    revealedKey = k
                    revealedSeed = if (hasSeed) viewModel.revealMnemonicWithBiometric(wallet) else null
                    isVerified = true
                }
            }
        )
    }
    Scaffold(topBar = {
        TopAppBar(
            title = { Text("Wallet Details") },
            navigationIcon = {
                IconButton(onClick = onDismiss) {
                    Icon(
                        Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = "Back"
                    )
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background)
        )
    }, containerColor = MaterialTheme.colorScheme.background) { padding ->
        if (!isVerified) {
            PinVerificationContent(
                title = "Enter PIN",
                subtitle = "Enter 6-digit PIN to reveal secrets",
                onVerify = { pin -> viewModel.verifyPin(pin) },
                onSuccess = { pin ->
                    val k = viewModel.revealPrivateKey(wallet, pin)
                    if (k != null) {
                        revealedKey = k
                        revealedSeed = if (hasSeed) viewModel.revealMnemonic(wallet, pin) else null
                        isVerified = true
                    }
                },
                biometricEnabled = canUseBiometric,
                onBiometric = ::onBiometric,
                modifier = Modifier.fillMaxSize().padding(padding)
            )
        } else {
            Column(
                Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp)
            ) {
                DetailSectionCard(glass = true) {
                    CurveBadge(curveInfo)
                    Spacer(Modifier.height(8.dp))
                    Text(
                        curveInfo.displayName,
                        style = MaterialTheme.typography.titleLarge.copy(
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold
                        )
                    )
                    Text(
                        curveInfo.description,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                        shape = RoundedCornerShape(8.dp)
                    ) {
                        Text(
                            "Curve ID: ${wallet.curveType}",
                            style = MaterialTheme.typography.labelSmall.copy(fontFamily = FontFamily.Monospace),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp)
                        )
                    }
                }
                DetailSectionCard {
                    Text(
                        "Public Address",
                        style = MaterialTheme.typography.titleSmall.copy(fontWeight = FontWeight.Bold),
                        color = MaterialTheme.colorScheme.primary
                    )
                    Surface(
                        color = MaterialTheme.colorScheme.surfaceContainerHighest,
                        shape = RoundedCornerShape(16.dp),
                        modifier = Modifier.fillMaxWidth(),
                        border = androidx.compose.foundation.BorderStroke(
                            1.5.dp,
                            MaterialTheme.colorScheme.outline.copy(alpha = 0.3f)
                        )
                    ) {
                        Text(
                            wallet.address,
                            style = MaterialTheme.typography.bodyMedium.copy(
                                fontFamily = FontFamily.Monospace,
                                letterSpacing = 0.5.sp,
                                fontWeight = FontWeight.SemiBold
                            ),
                            color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.padding(16.dp)
                        )
                    }
                    LoadingButton(
                        onClick = { copyToClipboard(ctx, wallet.address, toast = "Address copied") },
                        text = "Copy Address",
                        icon = Icons.Default.Public,
                        modifier = Modifier.fillMaxWidth(),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = KanariColors.Lime,
                            contentColor = KanariColors.Ink
                        )
                    )
                }
                Spacer(Modifier.height(8.dp)) // Extra spacing
                DetailSectionCard {
                    SecretRevealCard(
                        title = "Private Key",
                        secret = revealedKey,
                        isVisible = keyVisible,
                        onToggleVisibility = { keyVisible = !keyVisible },
                        onCopy = {
                            copyToClipboard(
                                ctx,
                                revealedKey!!,
                                label = "Private Key",
                                toast = "Private key copied"
                            )
                        }
                    )
                    SecurityWarningCard("Never share your private key")
                }
                DetailSectionCard {
                    if (!hasSeed) {
                        Text(
                            "This wallet was imported from private key - no seed phrase available",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    } else {
                        SecretRevealCard(
                            title = "Seed Phrase",
                            secret = revealedSeed,
                            isVisible = seedVisible,
                            onToggleVisibility = { seedVisible = !seedVisible },
                            onCopy = {
                                copyToClipboard(
                                    ctx,
                                    revealedSeed!!,
                                    label = "Seed",
                                    toast = "Seed phrase copied"
                                )
                            }
                        )
                        SecurityWarningCard("Never share your seed phrase")
                    }
                }
                LoadingButton(
                    onClick = { isVerified = false; revealedKey = null; revealedSeed = null },
                    text = "Lock & Protect",
                    icon = Icons.Default.Security,
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.buttonColors(
                        containerColor = MaterialTheme.colorScheme.surfaceVariant,
                        contentColor = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                )

                Spacer(Modifier.height(12.dp))

                TextButton(
                    onClick = onDeleteRequest,
                    modifier = Modifier.fillMaxWidth(),
                    colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error.copy(alpha = 0.8f))
                ) {
                    Icon(Icons.Default.Delete, contentDescription = null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("Delete Wallet Permanent", style = MaterialTheme.typography.labelLarge)
                }
            }
        }
    }
}