package com.kanari.kanari_crypto.compose

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.kanari.kanari_crypto.KanariCrypto
import com.kanari.kanari_crypto.model.CurveInfoModel
import com.kanari.kanari_crypto.model.KeyPairModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyGenerationScreen(
    modifier: Modifier = Modifier,
    defaultCurve: String = KanariCrypto.DEFAULT_CURVE,
    onKeyPairGenerated: ((KeyPairModel) -> Unit)? = null,
) {
    val snackbarHostState = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()

    var curves by remember { mutableStateOf<List<CurveInfoModel>>(emptyList()) }
    var selectedCurveInfo by remember { mutableStateOf<CurveInfoModel?>(null) }
    var mnemonic by remember { mutableStateOf<String?>(null) }
    var keyPairs by remember { mutableStateOf<List<KeyPairModel>>(emptyList()) }
    var isLoading by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }

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
            TopAppBar(title = { Text("Kanari Wallet") })
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .navigationBarsPadding()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "Select curve",
                style = MaterialTheme.typography.titleMedium,
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

            // แสดงส่วนตั้งค่าเพิ่มเติมเฉพาะกลุ่ม Classic/Hybrid ที่รองรับ Mnemonic
            if (selectedCurveInfo?.isPostQuantum == false || selectedCurveInfo?.isHybrid == true) {
                OutlinedTextField(
                    value = derivationPath,
                    onValueChange = { derivationPath = it },
                    label = { Text("Derivation Path (e.g. m/44'/0'/0'/0/0)") },
                    modifier = Modifier.fillMaxWidth()
                )

                OutlinedTextField(
                    value = if (addressCount == 0) "" else addressCount.toString(),
                    onValueChange = { addressCount = it.toIntOrNull() ?: 0 },
                    label = { Text("Number of addresses to derive") },
                    modifier = Modifier.fillMaxWidth()
                )
            }

            Button(
                onClick = {
                    scope.launch {
                        isLoading = true
                        val currentCurve = selectedCurveInfo?.name ?: defaultCurve
                        // ถ้าเป็น Curve กลุ่ม PQ (แบบเดี่ยว) จะสร้าง Keypair โดยตรง
                        val isPqOnly = selectedCurveInfo?.isPostQuantum == true && selectedCurveInfo?.isHybrid == false
                        
                        runCatching {
                            calculateWithLargeStack {
                                if (isPqOnly) {
                                    val pair = KanariCrypto.generateKeypair(currentCurve)
                                    null to listOf(pair)
                                } else {
                                    val words = KanariCrypto.generateMnemonic(12)
                                    val pairs = if (addressCount > 1) {
                                        // ใช้ฟังก์ชัน deriveMultipleAddresses
                                        KanariCrypto.deriveMultipleAddresses(words, derivationPath, currentCurve, addressCount)
                                    } else {
                                        // ใช้ฟังก์ชัน deriveKeypairFromPath (หรือ default ถ้า path ว่าง)
                                        val path = derivationPath.ifEmpty { "m/44'/0'/0'/0/0" }
                                        val pair = KanariCrypto.deriveKeypairFromPath(words, path, currentCurve)
                                        listOf(pair)
                                    }
                                    words to pairs
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
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Generate new wallet")
            }

            if (isLoading) {
                CircularProgressIndicator(modifier = Modifier.align(Alignment.CenterHorizontally))
            }

            mnemonic?.let { words ->
                Text(
                    text = "Recovery phrase",
                    style = MaterialTheme.typography.titleMedium,
                )
                MnemonicDisplay(mnemonic = words)
            }

            keyPairs.forEachIndexed { index, pair ->
                if (keyPairs.size > 1) {
                    Text(
                        text = "Address #${index + 1}",
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.Bold,
                        modifier = Modifier.padding(top = 8.dp)
                    )
                }

                WalletAddressCard(
                    address = pair.address,
                    onCopied = {
                        scope.launch {
                            snackbarHostState.showSnackbar("Address #${index + 1} copied")
                        }
                    },
                )

                WalletAddressCard(
                    address = pair.privateKey,
                    label = "Private Key (Hex)",
                    onCopied = {
                        scope.launch {
                            snackbarHostState.showSnackbar("Private key #${index + 1} copied")
                        }
                    },
                )

                Text(
                    text = "Curve: ${pair.curveType}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            // เพิ่มช่องว่างด้านล่างสุดเพื่อให้เลื่อนดู Private Key ได้ถนัด
            Spacer(modifier = Modifier.height(32.dp))
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
