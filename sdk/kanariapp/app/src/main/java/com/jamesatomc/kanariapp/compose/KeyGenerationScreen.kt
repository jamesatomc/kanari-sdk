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
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.FileDownload
import androidx.compose.material.icons.filled.Save
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.jamesatomc.kanariapp.ui.components.*
import com.jamesatomc.kanariapp.ui.theme.LocalKanariGradients
import com.jamesatomc.kanariapp.wallet.WalletRecord
import com.jamesatomc.kanariapp.wallet.WalletStorage
import com.jamesatomc.kanariapp.wallet.WalletViewModel
import com.kanari.kanari_crypto.KanariCrypto
import com.kanari.kanari_crypto.model.CurveInfoModel
import com.kanari.kanari_crypto.model.KeyPairModel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

enum class ImportMethod { RECOVERY_PHRASE, PRIVATE_KEY }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun KeyGenerationScreen(
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    defaultCurve: String = KanariCrypto.DEFAULT_CURVE,
    onKeyPairGenerated: ((KeyPairModel) -> Unit)? = null,
    viewModel: WalletViewModel? = null
) {
    val snackbarHostState = remember { SnackbarHostState() }
    val scope = rememberCoroutineScope()
    val scrollState = rememberScrollState()
    val context = LocalContext.current
    val walletStorage = remember { WalletStorage(context) }
    var curves by remember { mutableStateOf<List<CurveInfoModel>>(emptyList()) }
    var selectedCurveInfo by remember { mutableStateOf<CurveInfoModel?>(null) }
    var mnemonic by remember { mutableStateOf<String?>(null) }
    var keyPairs by remember { mutableStateOf<List<KeyPairModel>>(emptyList()) }
    var isLoading by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    var showSaveDialog by remember { mutableStateOf(false) }
    var walletName by remember { mutableStateOf("My Wallet") }
    var selectedTab by remember { mutableIntStateOf(0) }
    val tabs = listOf("Generate New", "Import Existing")
    var importMethod by remember { mutableStateOf(ImportMethod.RECOVERY_PHRASE) }
    var importInput by remember { mutableStateOf("") }
    var derivationPath by remember { mutableStateOf("m/44'/637'/0'/0/0") }
    var addressCount by remember { mutableIntStateOf(1) }
    var wordCount by remember { mutableIntStateOf(12) }
    LaunchedEffect(Unit) {
        isLoading = true
        runCatching { withContext(Dispatchers.Default) { KanariCrypto.listSupportedCurves() } }.onSuccess {
            curves = it
        }.onFailure { errorMessage = it.message }
        isLoading = false
    }
    LaunchedEffect(curves) {
        if (curves.isNotEmpty() && selectedCurveInfo == null) selectedCurveInfo =
            curves.find { it.name == defaultCurve } ?: curves.first()
    }
    LaunchedEffect(errorMessage) { errorMessage?.let { snackbarHostState.showSnackbar(it); errorMessage = null } }
    Scaffold(modifier = modifier, snackbarHost = { SnackbarHost(snackbarHostState) }, topBar = {
        TopAppBar(
            title = {
                Text(
                    "Kanari Key Generator",
                    style = MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold)
                )
            },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(
                        Icons.AutoMirrored.Filled.ArrowBack,
                        contentDescription = "Back"
                    )
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(
                containerColor = MaterialTheme.colorScheme.background,
                titleContentColor = MaterialTheme.colorScheme.onBackground
            )
        )
    }, containerColor = MaterialTheme.colorScheme.background) { padding ->
        if (showSaveDialog) {
            Dialog(
                onDismissRequest = { showSaveDialog = false },
                properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)
            ) {
                Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    PinVerificationContent(
                        title = "Save Wallet",
                        subtitle = "Enter 6-digit PIN to encrypt your wallet",
                        onVerify = { pin -> !walletStorage.hasPin() || walletStorage.verifyPin(pin) },
                        onSuccess = { pin ->
                            scope.launch {
                                runCatching {
                                    val pair = keyPairs.first()
                                    val encryptedPk = walletStorage.encrypt(pair.privateKey, pin)
                                    val encryptedMnemonic = mnemonic?.let { walletStorage.encrypt(it, pin) }
                                    val record = WalletRecord(
                                        id = java.util.UUID.randomUUID().toString(),
                                        name = walletName,
                                        address = pair.address,
                                        curveType = pair.curveType,
                                        privateKeyEncrypted = encryptedPk,
                                        mnemonicEncrypted = encryptedMnemonic
                                    )
                                    val existing = walletStorage.loadWallets().toMutableList()
                                    existing.add(record)
                                    walletStorage.saveWallets(existing)
                                    if (!walletStorage.hasPin()) walletStorage.savePin(pin)
                                    viewModel?.loadWallets()
                                    snackbarHostState.showSnackbar("Wallet saved successfully!")
                                    showSaveDialog = false
                                    onBack()
                                }.onFailure { errorMessage = "Failed to save: ${it.message}" }
                            }
                        },
                        modifier = Modifier.fillMaxSize()
                    )
                }
            }
        }
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(scrollState).navigationBarsPadding().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp)
        ) {
            SmartTabRow(
                selectedTabIndex = selectedTab,
                tabs = tabs,
                onTabSelected = {
                    selectedTab = it
                    mnemonic = null
                    keyPairs = emptyList()
                    importInput = ""
                    errorMessage = null
                }
            )
            DetailSectionCard(glass = true) {
                Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text(
                        if (selectedTab == 0) "Wallet Settings" else "Import Details",
                        style = MaterialTheme.typography.titleLarge,
                        color = MaterialTheme.colorScheme.primary,
                        fontWeight = FontWeight.ExtraBold
                    )
                    HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.2f))
                    if (selectedTab == 1) {
                        Text(
                            "Import Method",
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        val importMethods = listOf(ImportMethod.RECOVERY_PHRASE, ImportMethod.PRIVATE_KEY)
                        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                            importMethods.forEachIndexed { index, method ->
                                SegmentedButton(
                                    shape = SegmentedButtonDefaults.itemShape(
                                        index = index,
                                        count = importMethods.size
                                    ),
                                    onClick = { importMethod = method; importInput = "" },
                                    selected = importMethod == method
                                ) { Text(if (method == ImportMethod.RECOVERY_PHRASE) "Phrase" else "Key") }
                            }
                        }
                        OutlinedTextField(
                            value = importInput,
                            onValueChange = { importInput = it },
                            label = { Text(if (importMethod == ImportMethod.RECOVERY_PHRASE) "Recovery Mnemonic" else "Private Key (Hex)") },
                            modifier = Modifier.fillMaxWidth(),
                            shape = RoundedCornerShape(12.dp),
                            minLines = if (importMethod == ImportMethod.RECOVERY_PHRASE) 3 else 1,
                            placeholder = { Text(if (importMethod == ImportMethod.RECOVERY_PHRASE) "Enter your 12 or 24 word mnemonic phrase..." else "Enter your private key hex string...") })
                    }
                    Text(
                        "Cryptographic Curve",
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    if (curves.isNotEmpty()) CurveSelector(
                        curves = curves,
                        selectedCurve = selectedCurveInfo?.name ?: defaultCurve,
                        onCurveSelected = { name -> selectedCurveInfo = curves.find { it.name == name } })
                    if (selectedTab == 0 && selectedCurveInfo?.isPostQuantum == false) {
                        Text(
                            "Mnemonic Length",
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                            listOf(12, 24).forEachIndexed { index, count ->
                                SegmentedButton(
                                    shape = SegmentedButtonDefaults.itemShape(index = index, count = 2),
                                    onClick = { wordCount = count },
                                    selected = wordCount == count
                                ) { Text("$count words") }
                            }
                        }
                    }
                    val showPath =
                        (selectedTab == 0 && selectedCurveInfo?.isPostQuantum == false) || (selectedTab == 1 && importMethod == ImportMethod.RECOVERY_PHRASE)
                    if (showPath) OutlinedTextField(
                        value = derivationPath,
                        onValueChange = { derivationPath = it },
                        label = { Text("Derivation Path") },
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(12.dp)
                    )
                    if (selectedTab == 0 && selectedCurveInfo?.isPostQuantum == false) OutlinedTextField(
                        value = if (addressCount == 0) "" else addressCount.toString(),
                        onValueChange = { addressCount = it.toIntOrNull() ?: 0 },
                        label = { Text("Address Count") },
                        modifier = Modifier.fillMaxWidth(),
                        shape = RoundedCornerShape(12.dp)
                    )
                }
            }
            LoadingButton(
                onClick = {
                    scope.launch {
                        isLoading = true
                        val currentCurve = selectedCurveInfo?.name ?: defaultCurve
                        val supportsMnemonic = selectedCurveInfo?.isPostQuantum == false
                        runCatching {
                            if (selectedTab == 0) {
                                if (!supportsMnemonic) {
                                    val pair = KanariCrypto.generateKeypair(currentCurve)
                                    null to listOf(pair)
                                } else {
                                    val words = KanariCrypto.generateMnemonic(wordCount)
                                    val pairs = if (addressCount > 1) KanariCrypto.deriveMultipleAddresses(
                                        words,
                                        derivationPath,
                                        currentCurve,
                                        addressCount
                                    ) else {
                                        val path = derivationPath.ifEmpty { "m/44'/0'/0'/0/0" };
                                        val pair =
                                            KanariCrypto.deriveKeypairFromPath(words, path, currentCurve); listOf(
                                            pair
                                        )
                                    }
                                    words to pairs
                                }
                            } else {
                                if (importMethod == ImportMethod.RECOVERY_PHRASE) {
                                    val path = derivationPath.ifEmpty { "m/44'/637'/0'/0/0" }
                                    val pair = KanariCrypto.deriveKeypairFromPath(importInput, path, currentCurve)
                                    importInput to listOf(pair)
                                } else {
                                    val pair = KanariCrypto.importKeypairFromPrivateKey(importInput, currentCurve)
                                    null to listOf(pair)
                                }
                            }
                        }.onSuccess { (words, pairs) ->
                            mnemonic = words; keyPairs = pairs; pairs.firstOrNull()
                            ?.let { onKeyPairGenerated?.invoke(it) }
                        }.onFailure { errorMessage = it.message ?: "Operation failed" }
                        isLoading = false
                    }
                },
                isLoading = isLoading,
                text = if (selectedTab == 0) "Generate Secure Wallet" else "Import Existing Wallet",
                icon = if (selectedTab == 0) Icons.Default.Refresh else Icons.Default.FileDownload,
                modifier = Modifier.fillMaxWidth()
            )
            if (mnemonic != null || keyPairs.isNotEmpty()) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        if (selectedTab == 0) "Generated Results" else "Import Results",
                        style = MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold),
                        color = MaterialTheme.colorScheme.onBackground
                    )
                    Button(
                        onClick = { showSaveDialog = true },
                        colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.tertiary)
                    ) {
                        Icon(
                            Icons.Default.Save,
                            contentDescription = null
                        ); Spacer(Modifier.size(8.dp)); Text("Save Wallet")
                    }
                }
                mnemonic?.let { words ->
                    ElevatedCard(Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) {
                        Column(Modifier.padding(16.dp)) {
                            Text(
                                "Recovery Mnemonic",
                                style = MaterialTheme.typography.titleSmall,
                                color = MaterialTheme.colorScheme.primary,
                                modifier = Modifier.padding(bottom = 12.dp)
                            )
                            MnemonicDisplay(mnemonic = words)
                            Text(
                                "Keep this phrase secret and safe. It can recover your entire wallet.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.error,
                                modifier = Modifier.padding(top = 12.dp)
                            )
                        }
                    }
                }
                keyPairs.forEachIndexed { index, pair ->
                    ElevatedCard(Modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp)) {
                        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                            Row(
                                Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    if (keyPairs.size > 1) "Address #${index + 1}" else "Wallet Details",
                                    style = MaterialTheme.typography.titleSmall,
                                    color = MaterialTheme.colorScheme.primary
                                )
                                Text(
                                    pair.curveType,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                            WalletAddressCard(
                                address = pair.address,
                                label = "Public Address",
                                onCopied = { scope.launch { snackbarHostState.showSnackbar("Address copied") } })
                            WalletAddressCard(
                                address = pair.privateKey,
                                label = "Private Key (Hex)",
                                onCopied = { scope.launch { snackbarHostState.showSnackbar("Private key copied") } })
                        }
                    }
                }
            }
            Spacer(Modifier.height(40.dp))
        }
    }
}
