package com.jamesatomc.kanariapp.wallet

import android.app.Application
import android.content.Context
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.jamesatomc.kanariapp.network.KanariClient
import com.jamesatomc.kanariapp.network.models.AccountInfo
import com.jamesatomc.kanariapp.network.models.TokenBalance
import com.jamesatomc.kanariapp.network.models.TransactionDetails
import com.jamesatomc.kanariapp.network.models.KanariEnvironment
import com.jamesatomc.kanariapp.ui.theme.ThemeMode
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import androidx.core.content.edit

class WalletViewModel(application: Application) : AndroidViewModel(application) {
    private val walletStorage = WalletStorage(application)

    private val _wallets = MutableStateFlow<List<WalletRecord>>(emptyList())
    val wallets: StateFlow<List<WalletRecord>> = _wallets.asStateFlow()

    private val _activeWallet = MutableStateFlow<WalletRecord?>(null)
    val activeWallet: StateFlow<WalletRecord?> = _activeWallet.asStateFlow()

    private val _accountInfo = MutableStateFlow<AccountInfo?>(null)
    val accountInfo: StateFlow<AccountInfo?> = _accountInfo.asStateFlow()

    private val _tokenBalances = MutableStateFlow<List<TokenBalance>>(emptyList())
    val tokenBalances: StateFlow<List<TokenBalance>> = _tokenBalances.asStateFlow()

    private val _transactions = MutableStateFlow<List<TransactionDetails>>(emptyList())
    val transactions: StateFlow<List<TransactionDetails>> = _transactions.asStateFlow()

    private val _isLoading = MutableStateFlow(false)
    val isLoading: StateFlow<Boolean> = _isLoading.asStateFlow()

    private val _error = MutableStateFlow<String?>(null)
    val error: StateFlow<String?> = _error.asStateFlow()

    private val _environment = MutableStateFlow(KanariEnvironment.dev)
    val environment: StateFlow<KanariEnvironment> = _environment.asStateFlow()

    private val _isUnlocked = MutableStateFlow(false)
    val isUnlocked: StateFlow<Boolean> = _isUnlocked.asStateFlow()

    private var unlockedPin: String? = null

    private var client = KanariClient(_environment.value)

    private val prefs = application.getSharedPreferences("kanari_prefs", Context.MODE_PRIVATE)

    private val _themeMode = MutableStateFlow(loadThemeMode())
    val themeMode: StateFlow<ThemeMode> = _themeMode.asStateFlow()

    private val _biometricEnabled = MutableStateFlow(walletStorage.isBiometricEnabled())
    val biometricEnabled: StateFlow<Boolean> = _biometricEnabled.asStateFlow()

    private fun loadThemeMode(): ThemeMode {
        val saved = prefs.getString("theme_mode", ThemeMode.SYSTEM.name) ?: ThemeMode.SYSTEM.name
        return try {
            ThemeMode.valueOf(saved)
        } catch (_: Exception) {
            ThemeMode.SYSTEM
        }
    }

    fun setThemeMode(mode: ThemeMode) {
        _themeMode.value = mode
        prefs.edit { putString("theme_mode", mode.name) }
    }

    init {
        loadWallets()
    }

    fun loadWallets() {
        viewModelScope.launch {
            _isLoading.value = true
            try {
                val records = walletStorage.loadWallets()
                _wallets.value = records
                if (records.isNotEmpty() && _activeWallet.value == null) {
                    _activeWallet.value = records.first()
                }
            } catch (e: Exception) {
                _error.value = "Failed to load wallets: ${e.message}"
            }
            _isLoading.value = false
        }
    }

    fun setEnvironment(env: KanariEnvironment) {
        _environment.value = env
        client = KanariClient(env)
        refreshBalance()
    }

    fun refreshBalance() {
        val address = _activeWallet.value?.address ?: return
        viewModelScope.launch {
            _isLoading.value = true
            _error.value = null

            try {
                val info = client.getAccount(address)
                _accountInfo.value = info

                val balances = client.getAllBalances(address)
                _tokenBalances.value = balances

                val txs = client.getAllTransactions(address)
                _transactions.value = txs
            } catch (e: Exception) {
                // Ignore "Owner not found" as a hard error for new wallets
                if (e.message?.contains("Owner not found", ignoreCase = true) == true) {
                    _accountInfo.value = null
                    _tokenBalances.value = emptyList()
                    _transactions.value = emptyList()
                } else {
                    _error.value = "Failed to refresh balance: ${e.message}"
                }
            }
            _isLoading.value = false
        }
    }

    fun switchWallet(record: WalletRecord) {
        _activeWallet.value = record
        refreshBalance()
    }

    fun unlock(pin: String): Boolean {
        if (walletStorage.verifyPin(pin)) {
            unlockedPin = pin
            _isUnlocked.value = true
            // Sync biometric PIN if biometric is enabled (like kanari_pay)
            if (walletStorage.isBiometricEnabled()) {
                try {
                    walletStorage.saveBiometricPin(pin)
                } catch (_: Exception) {
                }
            }
            loadWallets()
            return true
        }
        return false
    }

    fun unlockWithBiometric(): Boolean {
        val pin = walletStorage.getBiometricPin() ?: return false
        return unlock(pin)
    }

    fun revealPrivateKeyWithBiometric(record: WalletRecord): String? {
        val pin = walletStorage.getBiometricPin() ?: return null
        return revealPrivateKey(record, pin)
    }

    fun revealMnemonicWithBiometric(record: WalletRecord): String? {
        val pin = walletStorage.getBiometricPin() ?: return null
        return revealMnemonic(record, pin)
    }

    fun verifyPin(pin: String): Boolean = walletStorage.verifyPin(pin)

    fun revealPrivateKey(record: WalletRecord, pin: String): String? {
        if (!verifyPin(pin)) return null
        val enc = record.privateKeyEncrypted ?: return null
        return try {
            walletStorage.decrypt(enc, pin)
        } catch (_: Exception) {
            null
        }
    }

    fun revealMnemonic(record: WalletRecord, pin: String): String? {
        if (!verifyPin(pin)) return null
        val enc = record.mnemonicEncrypted ?: return null
        return try {
            walletStorage.decrypt(enc, pin)
        } catch (_: Exception) {
            null
        }
    }

    fun isBiometricEnabled(): Boolean = walletStorage.isBiometricEnabled()

    fun setBiometricEnabled(enabled: Boolean): Boolean {
        val result = if (enabled) {
            val pin = unlockedPin ?: return false
            walletStorage.saveBiometricPin(pin)
            true
        } else {
            walletStorage.clearBiometricPin()
            walletStorage.setBiometricEnabled(false)
            true
        }
        _biometricEnabled.value = walletStorage.isBiometricEnabled()
        return result
    }

    fun changePin(oldPin: String, newPin: String): Boolean {
        if (!walletStorage.verifyPin(oldPin)) return false
        if (newPin.length != 6 || !newPin.all { it.isDigit() }) return false
        return try {
            val wallets = walletStorage.loadWallets()
            val reEncrypted = wallets.map { record ->
                val newPrivate = record.privateKeyEncrypted?.let {
                    try {
                        walletStorage.decrypt(it, oldPin)
                    } catch (_: Exception) {
                        null
                    }
                }?.let { walletStorage.encrypt(it, newPin) } ?: record.privateKeyEncrypted
                val newMnemonic = record.mnemonicEncrypted?.let {
                    try {
                        walletStorage.decrypt(it, oldPin)
                    } catch (_: Exception) {
                        null
                    }
                }?.let { walletStorage.encrypt(it, newPin) } ?: record.mnemonicEncrypted
                record.copy(privateKeyEncrypted = newPrivate, mnemonicEncrypted = newMnemonic)
            }
            walletStorage.saveWallets(reEncrypted)
            walletStorage.savePin(newPin)
            unlockedPin = newPin
            if (walletStorage.isBiometricEnabled()) {
                walletStorage.saveBiometricPin(newPin)
            }
            _wallets.value = reEncrypted
            _activeWallet.value = reEncrypted.find { it.id == _activeWallet.value?.id } ?: reEncrypted.firstOrNull()
            true
        } catch (_: Exception) {
            false
        }
    }

    fun logout() {
        _activeWallet.value = null
        _accountInfo.value = null
        _wallets.value = emptyList()
    }

    fun deleteWallet(record: WalletRecord) {
        viewModelScope.launch {
            val current = _wallets.value.toMutableList()
            current.remove(record)
            walletStorage.saveWallets(current)
            _wallets.value = current
            if (_activeWallet.value?.id == record.id) {
                _activeWallet.value = current.firstOrNull()
                refreshBalance()
            }
        }
    }

    suspend fun transfer(recipient: String, amount: ULong, tokenType: String = "0x2::kanari::KANARI"): Boolean {
        val record = _activeWallet.value ?: run {
            _error.value = "No active wallet"
            return false
        }
        val pin = unlockedPin ?: run {
            _error.value = "Wallet is locked. Please unlock again."
            return false
        }

        return try {
            val privateKeyEncrypted = record.privateKeyEncrypted ?: run {
                _error.value = "Wallet missing private key"
                return false
            }
            val isKanari = tokenType == "0x2::kanari::KANARI" ||
                    tokenType.lowercase(java.util.Locale.US) == "0x2::kanari::kanari" ||
                    tokenType.endsWith("::kanari::KANARI", ignoreCase = true)

            val privateKey = walletStorage.decrypt(privateKeyEncrypted, pin)
            val wallet = KanariWallet.fromPrivateKey(privateKey, record.curveType)

            val result = if (isKanari) {
                client.transfer(wallet, recipient, amount)
            } else {
                // Generic fungible token transfer via kanari_buildTokenTransfer / kanari_callFunction
                // Amount is already in token's smallest units (as per parseAmountToMist)
                client.transferToken(wallet, recipient, tokenType, amount)
            }
            val status = result?.status?.lowercase(java.util.Locale.US)
            val isSuccess = result?.success == true ||
                    status == "success" || status == "executed" ||
                    status == "committed" || status == "pending" ||
                    status == "simulated_pending"

            if (isSuccess) {
                _error.value = null
                refreshBalance()
                true
            } else {
                val msg = result?.errorMessage ?: "Transaction failed (Status: ${result?.status})"
                _error.value = mapTransferError(msg)
                false
            }
        } catch (e: Exception) {
            _error.value = mapTransferError(e.message ?: "Unknown error")
            false
        }
    }

    private fun mapTransferError(raw: String): String {
        val text = raw.lowercase(java.util.Locale.US)
        return when {
            text.contains("unexpected call_error") || text.contains("call_error") ->
                "Transfer failed: Local crypto error (Unexpected CALL_ERROR). This was a bug in BCS signing that is now fixed. Please retry. If it persists, re-import wallet and ensure curve type is correct."

            text.contains("invalid transaction signature") || text.contains("invalid_transaction_signature") ->
                "Transfer failed: Invalid transaction signature. BCS hash mismatch - please update app and retry."

            text.contains("requires two distinct") || text.contains("native_transfer_policy_not_satisfied") ||
                    text.contains("allows_single_coin_for_transfer_and_gas") ->
                "Native KANARI transfer needs two separate Coin<KANARI> objects: one coin to send and another coin to pay gas. Receive/fund this wallet once more, then try again."

            text.contains("no single coin") && text.contains("can cover requested amount") ->
                "Native KANARI transfer needs one Coin<KANARI> with enough balance to cover amount. Try a smaller amount or fund another coin."

            text.contains("no spendable native gas coin") ->
                "Insufficient gas coin. Need a separate Coin<KANARI> with enough Mist to pay gas (gas_limit * gas_price)."

            text.contains("missing a valid nonce") || text.contains("nonce must be non-zero") ->
                "Transfer failed: Transaction nonce error. Please try again."

            raw.startsWith("Transfer failed:") -> raw
            else -> "Transfer failed: $raw"
        }
    }
}