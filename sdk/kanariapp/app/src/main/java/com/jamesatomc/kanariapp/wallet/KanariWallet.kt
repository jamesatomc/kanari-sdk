package com.jamesatomc.kanariapp.wallet

import com.kanari.kanari_crypto.KanariCrypto
import com.kanari.kanari_crypto.model.KeyPairModel

class KanariWallet(
    val keyPair: KeyPairModel
) {
    val address: String get() = keyPair.address
    val taggedAddress: String get() = keyPair.taggedAddress
    val privateKey: String get() = keyPair.privateKey
    val curveType: String get() = keyPair.curveType

    companion object {

        suspend fun fromPrivateKey(
            privateKey: String,
            curveName: String = KanariCrypto.DEFAULT_CURVE
        ): KanariWallet {
            val pair = KanariCrypto.importKeypairFromPrivateKey(privateKey, curveName)
            return KanariWallet(pair)
        }
    }

    suspend fun sign(message: ByteArray): ByteArray {
        return KanariCrypto.signMessage(privateKey, message, curveType)
    }
}