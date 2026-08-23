package com.jamesatomc.kanariapp.compose

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Tab
import androidx.compose.material3.TabRow
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.kanari.kanari_crypto.KanariCrypto
import com.kanari.kanari_crypto.model.CurveInfoModel
import com.kanari.kanari_crypto.model.KeyPairModel

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

enum class ImportMethod {
    RECOVERY_PHRASE, PRIVATE_KEY
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyGenerationScreen(
    modifier: Modifier = Modifier,
    defaultCurve: String = KanariCrypto.DEFAULT_CURVE,
    onKeyPairGenerated: ((KeyPairModel) -> Unit)? = null,
) {
    val snackbarHostState = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val scrollState = rememberScrollState()

    var curves by remember { mutableStateOf<List<CurveInfoModel>>(emptyList()) }
    var selectedCurveInfo by remember { mutableStateOf<CurveInfoModel?>(null) }
    var mnemonic by remember { mutableStateOf<String?>(null) }
    var keyPairs by remember { mutableStateOf<List<KeyPairModel>>(emptyList()) }
    var isLoading by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }

    // Mode options
    var selectedTab by remember { mutableIntStateOf(0) }
    val tabs = listOf("Generate New", "Import Existing")
    
    var importMethod by remember { mutableStateOf(ImportMethod.RECOVERY_PHRASE) }
    var importInput by remember { mutableStateOf("") }

    // Advanced options
    var derivationPath by remember { mutableStateOf("m/44'/637'/0'/0/0") }
    var addressCount by remember { mutableIntStateOf(1) }

    LaunchedEffect(Unit) {
        isLoading = true
        runCatching {
            withContext(Dispatchers.Default) {
                KanariCrypto.listSupportedCurves()
            }
        }.onSuccess { curves = it }
            .onFailure { errorMessage = it.message }
        isLoading = false
    }

    LaunchedEffect(curves) {
        if (curves.isNotEmpty() && selectedCurveInfo == null) {
            selectedCurveInfo = curves.find { it.name == defaultCurve } ?: curves.first()
        }
    }

    LaunchedEffect(errorMessage) {
        errorMessage?.let {
            snackbarHostState.showSnackbar(it)
            errorMessage = null
        }
    }

    Scaffold(
        modifier = modifier,
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = { 
                    Text(
                        "Kanari Key Generator", 
                        style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold)
                    ) 
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    titleContentColor = MaterialTheme.colorScheme.onBackground
                )
            )
        },
        containerColor = MaterialTheme.colorScheme.background
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(scrollState)
                .navigationBarsPadding()
                .padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            // TabRow for mode switching
            TabRow(
                selectedTabIndex = selectedTab,
                containerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f),
                contentColor = MaterialTheme.colorScheme.primary,
                divider = {},
                modifier = Modifier.clip(RoundedCornerShape(12.dp))
            ) {
                tabs.forEachIndexed { index, title ->
                    Tab(
                        selected = selectedTab == index,
                        onClick = {
                            selectedTab = index
                            mnemonic = null
                            keyPairs = emptyList()
                            importInput = ""
                            errorMessage = null
                        },
                        text = {
                            Text(
                                text = title,
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = if (selectedTab == index) FontWeight.Bold else FontWeight.Normal,
                                modifier = Modifier.padding(vertical = 12.dp)
                            )
                        }
                    )
                }
            }

            // Section 1: Settings & Input
            ElevatedCard(
                modifier = Modifier.fillMaxWidth(),
                shape = RoundedCornerShape(16.dp),
            ) {
                Column(
                    modifier = Modifier.padding(20.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp)
                ) {
                    Text(
                        text = if (selectedTab == 0) "Wallet Settings" else "Import Details",
                        style = MaterialTheme.typography.titleMedium,
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.Bold
                    )
                    
                    HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.2f))

                    // Import Method Selection
                    if (selectedTab == 1) {
                        Text(
                            text = "Import Method",
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )

                        val importMethods = listOf(ImportMethod.RECOVERY_PHRASE, ImportMethod.PRIVATE_KEY)
                        SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
                            importMethods.forEachIndexed { index, method ->
                                SegmentedButton(
                                    shape = SegmentedButtonDefaults.itemShape(index = index, count = importMethods.size),
                                    onClick = { 
                                        importMethod = method
                                        importInput = ""
                                    },
                                    selected = importMethod == method
                                ) {
                                    Text(if (method == ImportMethod.RECOVERY_PHRASE) "Phrase" else "Key")
                                }
                            }
                        }

                        OutlinedTextField(
                            value = importInput,
                            onValueChange = { importInput = it },
                            label = { Text(if (importMethod == ImportMethod.RECOVERY_PHRASE) "Recovery Mnemonic" else "Private Key (Hex)") },
                            modifier = Modifier.fillMaxWidth(),
                            shape = RoundedCornerShape(12.dp),
                            minLines = if (importMethod == ImportMethod.RECOVERY_PHRASE) 3 else 1,
                            placeholder = {
                                Text(
                                    if (importMethod == ImportMethod.RECOVERY_PHRASE) 
                                        "Enter your 12 or 24 word mnemonic phrase..." 
                                    else "Enter your private key hex string..."
                                )
                            }
                        )
                    }

                    Text(
                        text = "Cryptographic Curve",
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )

                    if (curves.isNotEmpty()) {
                        CurveSelector(
                            curves = curves,
                            selectedCurve = selectedCurveInfo?.name ?: defaultCurve,
                            onCurveSelected = { name ->
                                selectedCurveInfo = curves.find { it.name == name }
                            },
                        )
                    }

                    // Advanced Options / Derivation Path
                    val showPath = (selectedTab == 0 && selectedCurveInfo?.isPostQuantum == false) ||
                            (selectedTab == 1 && importMethod == ImportMethod.RECOVERY_PHRASE)
                    
                    if (showPath) {
                        OutlinedTextField(
                            value = derivationPath,
                            onValueChange = { derivationPath = it },
                            label = { Text("Derivation Path") },
                            modifier = Modifier.fillMaxWidth(),
                            shape = RoundedCornerShape(12.dp)
                        )
                    }

                    if (selectedTab == 0 && selectedCurveInfo?.isPostQuantum == false) {
                        OutlinedTextField(
                            value = if (addressCount == 0) "" else addressCount.toString(),
                            onValueChange = { addressCount = it.toIntOrNull() ?: 0 },
                            label = { Text("Address Count") },
                            modifier = Modifier.fillMaxWidth(),
                            shape = RoundedCornerShape(12.dp)
                        )
                    }
                }
            }

            // Section 2: Action Button
            Button(
                onClick = {
                    scope.launch {
                        isLoading = true
                        val currentCurve = selectedCurveInfo?.name ?: defaultCurve
                        val supportsMnemonic = selectedCurveInfo?.isPostQuantum == false
                        
                        runCatching {
                            calculateWithLargeStack {
                                if (selectedTab == 0) {
                                    // Generate logic
                                    if (!supportsMnemonic) {
                                        val pair = KanariCrypto.generateKeypair(currentCurve)
                                        null to listOf(pair)
                                    } else {
                                        val words = KanariCrypto.generateMnemonic(12)
                                        val pairs = if (addressCount > 1) {
                                            KanariCrypto.deriveMultipleAddresses(words, derivationPath, currentCurve, addressCount)
                                        } else {
                                            val path = derivationPath.ifEmpty { "m/44'/0'/0'/0/0" }
                                            val pair = KanariCrypto.deriveKeypairFromPath(words, path, currentCurve)
                                            listOf(pair)
                                        }
                                        words to pairs
                                    }
                                } else {
                                    // Import logic
                                    if (importMethod == ImportMethod.RECOVERY_PHRASE) {
                                        val path = derivationPath.ifEmpty { "m/44'/637'/0'/0/0" }
                                        val pair = KanariCrypto.deriveKeypairFromPath(importInput, path, currentCurve)
                                        null to listOf(pair)
                                    } else {
                                        val pair = KanariCrypto.importKeypairFromPrivateKey(importInput, currentCurve)
                                        null to listOf(pair)
                                    }
                                }
                            }
                        }.onSuccess { (words, pairs) ->
                            mnemonic = words
                            keyPairs = pairs
                            pairs.firstOrNull()?.let { onKeyPairGenerated?.invoke(it) }
                        }.onFailure {
                            errorMessage = it.message ?: "Operation failed"
                        }
                        isLoading = false
                    }
                },
                enabled = !isLoading,
                modifier = Modifier
                    .fillMaxWidth()
                    .height(64.dp),
                shape = RoundedCornerShape(16.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary
                ),
                contentPadding = PaddingValues(16.dp)
            ) {
                if (isLoading) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(24.dp),
                        color = MaterialTheme.colorScheme.onPrimary,
                        strokeWidth = 3.dp
                    )
                } else {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(
                            if (selectedTab == 0) Icons.Default.Refresh else Icons.Default.FileDownload, 
                            contentDescription = null
                        )
                        Spacer(Modifier.size(8.dp))
                        Text(
                            if (selectedTab == 0) "Generate Secure Wallet" else "Import Existing Wallet",
                            style = MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.ExtraBold)
                        )
                    }
                }
            }

            // Section 3: Results
            if (mnemonic != null || keyPairs.isNotEmpty()) {
                Text(
                    text = if (selectedTab == 0) "Generated Results" else "Import Results",
                    style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold),
                    color = MaterialTheme.colorScheme.onBackground
                )

                mnemonic?.let { words ->
                    ElevatedCard(
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(16.dp)
                    ) {
                        Column(modifier = Modifier.padding(20.dp)) {
                            Text(
                                text = "Recovery Mnemonic",
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.primary,
                                modifier = Modifier.padding(bottom = 12.dp)
                            )
                            MnemonicDisplay(mnemonic = words)
                            Text(
                                text = "Keep this phrase secret and safe. It can recover your entire wallet.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.error,
                                modifier = Modifier.padding(top = 12.dp)
                            )
                        }
                    }
                }

                keyPairs.forEachIndexed { index, pair ->
                    ElevatedCard(
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(16.dp)
                    ) {
                        Column(
                            modifier = Modifier.padding(20.dp),
                            verticalArrangement = Arrangement.spacedBy(16.dp)
                        ) {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    text = if (keyPairs.size > 1) "Address #${index + 1}" else "Wallet Details",
                                    style = MaterialTheme.typography.titleSmall,
                                    color = MaterialTheme.colorScheme.primary
                                )
                                Text(
                                    text = pair.curveType,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                            
                            WalletAddressCard(
                                address = pair.address,
                                label = "Public Address",
                                onCopied = {
                                    scope.launch { snackbarHostState.showSnackbar("Address copied") }
                                }
                            )

                            WalletAddressCard(
                                address = pair.privateKey,
                                label = "Private Key (Hex)",
                                onCopied = {
                                    scope.launch { snackbarHostState.showSnackbar("Private key copied") }
                                }
                            )
                        }
                    }
                }
            }

            Spacer(modifier = Modifier.height(40.dp))
        }
    }
}

/**
 * ฟังก์ชันช่วยรันงาน Crypto บนเธรดใหม่ที่มีขนาด Stack ใหญ่ขึ้น
 */
private suspend fun <T> calculateWithLargeStack(block: () -> T): T = suspendCancellableCoroutine { cont ->
    val thread = Thread(null, {
        try {
            cont.resume(block())
        } catch (e: Throwable) {
            cont.resumeWithException(e)
        }
    }, "CryptoThread", 16 * 1024 * 1024) // เพิ่มเป็น 16MB เพื่อความชัวร์สำหรับ Hybrid PQ

    thread.start()
}
