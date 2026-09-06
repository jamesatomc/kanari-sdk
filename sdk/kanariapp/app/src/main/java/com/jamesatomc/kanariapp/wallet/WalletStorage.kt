package com.jamesatomc.kanariapp.wallet

import android.content.Context
import android.util.Base64
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKeys
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.PBEKeySpec
import javax.crypto.spec.SecretKeySpec
import androidx.core.content.edit

@Serializable
data class EncryptedData(
    val salt: String,
    val nonce: String,
    val cipherText: String,
    val mac: String,
    val iterations: Int,
    val algorithm: String = "aes_gcm_256_pbkdf2_sha256"
)

@Serializable
data class WalletRecord(
    val id: String,
    val name: String,
    val address: String,
    val curveType: String,
    val privateKeyEncrypted: EncryptedData? = null,
    val mnemonicEncrypted: EncryptedData? = null,
    val encryption: String = "pin_aes_gcm_pbkdf2_v1"
)

@Suppress("DEPRECATION")
class WalletStorage(context: Context) {
    private val sharedPrefs by lazy {
        val masterKeyAlias = MasterKeys.getOrCreate(MasterKeys.AES256_GCM_SPEC)
        EncryptedSharedPreferences.create(
            "kanari_secure_prefs",
            masterKeyAlias,
            context,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM
        )
    }

    private val json = Json { ignoreUnknownKeys = true }

    companion object {
        private const val KEY_WALLETS = "kanari_wallets"
        private const val KEY_PIN_SALT = "kanari_pin_salt"
        private const val KEY_PIN_VERIFIER = "kanari_pin_verifier"
        private const val KEY_BIOMETRIC_ENABLED = "kanari_biometric_enabled"
        private const val KEY_BIOMETRIC_PIN = "kanari_biometric_pin"
        private const val PIN_LENGTH = 6
        private const val KDF_ITERATIONS = 210000
    }

    fun hasPin(): Boolean = sharedPrefs.contains(KEY_PIN_VERIFIER)

    fun savePin(pin: String) {
        require(pin.length == PIN_LENGTH) { "PIN must be $PIN_LENGTH digits" }
        val salt = ByteArray(16).apply { SecureRandom().nextBytes(this) }
        val verifier = deriveKey(pin, salt)
        
        sharedPrefs.edit {
            putString(KEY_PIN_SALT, Base64.encodeToString(salt, Base64.NO_WRAP))
                .putString(KEY_PIN_VERIFIER, Base64.encodeToString(verifier, Base64.NO_WRAP))
        }
    }

    fun verifyPin(pin: String): Boolean {
        val saltBase64 = sharedPrefs.getString(KEY_PIN_SALT, null) ?: return false
        val verifierBase64 = sharedPrefs.getString(KEY_PIN_VERIFIER, null) ?: return false
        
        val salt = Base64.decode(saltBase64, Base64.NO_WRAP)
        val verifier = Base64.decode(verifierBase64, Base64.NO_WRAP)
        
        val candidate = deriveKey(pin, salt)
        return candidate.contentEquals(verifier)
    }

    fun isBiometricEnabled(): Boolean = sharedPrefs.getBoolean(KEY_BIOMETRIC_ENABLED, false)

    fun setBiometricEnabled(enabled: Boolean) {
        sharedPrefs.edit { putBoolean(KEY_BIOMETRIC_ENABLED, enabled) }
        if (!enabled) sharedPrefs.edit { remove(KEY_BIOMETRIC_PIN) }
    }

    fun saveBiometricPin(pin: String) {
        require(pin.length == PIN_LENGTH) { "PIN must be $PIN_LENGTH digits" }
        // Store PIN encrypted via EncryptedSharedPreferences (already encrypted at rest)
        sharedPrefs.edit { putString(KEY_BIOMETRIC_PIN, pin) }
        setBiometricEnabled(true)
    }

    fun getBiometricPin(): String? {
        if (!isBiometricEnabled()) return null
        return sharedPrefs.getString(KEY_BIOMETRIC_PIN, null)
    }

    fun clearBiometricPin() {
        sharedPrefs.edit { remove(KEY_BIOMETRIC_PIN).remove(KEY_BIOMETRIC_ENABLED) }
    }

    fun saveWallets(wallets: List<WalletRecord>) {
        val data = json.encodeToString(wallets)
        sharedPrefs.edit { putString(KEY_WALLETS, data) }
    }

    fun loadWallets(): List<WalletRecord> {
        val data = sharedPrefs.getString(KEY_WALLETS, null) ?: return emptyList()
        return try {
            json.decodeFromString(data)
        } catch (_: Exception) {
            emptyList()
        }
    }

    private fun deriveKey(pin: String, salt: ByteArray): ByteArray {
        val spec = PBEKeySpec(pin.toCharArray(), salt, KDF_ITERATIONS, 256)
        val factory = SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256")
        return factory.generateSecret(spec).encoded
    }

    fun encrypt(data: String, pin: String): EncryptedData {
        val salt = ByteArray(16).apply { SecureRandom().nextBytes(this) }
        val keyBytes = deriveKey(pin, salt)
        val key = SecretKeySpec(keyBytes, "AES")
        
        val nonce = ByteArray(12).apply { SecureRandom().nextBytes(this) }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key, GCMParameterSpec(128, nonce))
        
        val encryptedBytes = cipher.doFinal(data.toByteArray())
        
        // Split cipherText and MAC (Android's AES/GCM includes MAC at the end)
        val cipherText = encryptedBytes.copyOfRange(0, encryptedBytes.size - 16)
        val mac = encryptedBytes.copyOfRange(encryptedBytes.size - 16, encryptedBytes.size)
        
        return EncryptedData(
            salt = Base64.encodeToString(salt, Base64.NO_WRAP),
            nonce = Base64.encodeToString(nonce, Base64.NO_WRAP),
            cipherText = Base64.encodeToString(cipherText, Base64.NO_WRAP),
            mac = Base64.encodeToString(mac, Base64.NO_WRAP),
            iterations = KDF_ITERATIONS
        )
    }

    fun decrypt(encryptedData: EncryptedData, pin: String): String {
        val salt = Base64.decode(encryptedData.salt, Base64.NO_WRAP)
        val keyBytes = deriveKey(pin, salt)
        val key = SecretKeySpec(keyBytes, "AES")
        
        val nonce = Base64.decode(encryptedData.nonce, Base64.NO_WRAP)
        val cipherText = Base64.decode(encryptedData.cipherText, Base64.NO_WRAP)
        val mac = Base64.decode(encryptedData.mac, Base64.NO_WRAP)
        
        val combined = cipherText + mac
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, nonce))
        
        return String(cipher.doFinal(combined))
    }
}