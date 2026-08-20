package com.kanari.kanari_crypto

import com.kanari.kanari_crypto.model.CurveInfoModel
import com.kanari.kanari_crypto.model.KeyPairModel
import uniffi.kanari_kotlin.CurveInfo
import uniffi.kanari_kotlin.KeyPairData
import uniffi.kanari_kotlin.blake3HashApi
import uniffi.kanari_kotlin.deriveKeypairFromMnemonic as ffiDeriveKeypairFromMnemonic
import uniffi.kanari_kotlin.deriveKeypairFromPathApi
import uniffi.kanari_kotlin.deriveMultipleAddressesApi
import uniffi.kanari_kotlin.generateKeypairApi
import uniffi.kanari_kotlin.generateMnemonicApi
import uniffi.kanari_kotlin.importKeypairFromPrivateKey as ffiImportKeypairFromPrivateKey
import uniffi.kanari_kotlin.listSupportedCurves as ffiListSupportedCurves
import uniffi.kanari_kotlin.signMessageApi
import uniffi.kanari_kotlin.verifySignatureApi

/**
 * High-level entry point for Kanari cryptographic operations on Android.
 *
 * Wraps the UniFFI-generated Rust bindings with idiomatic Kotlin types.
 */
object KanariCrypto {
    const val DEFAULT_CURVE = "Ed25519"

    fun generateKeypair(curveName: String = DEFAULT_CURVE): KeyPairModel =
        generateKeypairApi(curveName).toModel()

    fun generateMnemonic(wordCount: Int = 12): String {
        require(wordCount == 12 || wordCount == 24) {
            "Only 12 or 24-word mnemonics are supported"
        }
        return generateMnemonicApi(wordCount.toUInt())
    }

    fun deriveKeypairFromMnemonic(
        mnemonic: String,
        curveName: String = DEFAULT_CURVE,
    ): KeyPairModel = ffiDeriveKeypairFromMnemonic(mnemonic, curveName).toModel()

    fun deriveKeypairFromPath(
        mnemonic: String,
        derivationPath: String,
        curveName: String = DEFAULT_CURVE,
    ): KeyPairModel =
        deriveKeypairFromPathApi(mnemonic, derivationPath, curveName).toModel()

    fun deriveMultipleAddresses(
        mnemonic: String,
        pathTemplate: String,
        curveName: String = DEFAULT_CURVE,
        count: Int,
    ): List<KeyPairModel> =
        deriveMultipleAddressesApi(mnemonic, pathTemplate, curveName, count.toUInt())
            .map { it.toModel() }

    fun importKeypairFromPrivateKey(
        privateKey: String,
        curveName: String = DEFAULT_CURVE,
    ): KeyPairModel =
        ffiImportKeypairFromPrivateKey(privateKey, curveName).toModel()

    fun signMessage(
        privateKey: String,
        message: ByteArray,
        curveName: String = DEFAULT_CURVE,
    ): ByteArray =
        signMessageApi(privateKey, message.toUByteList(), curveName).toByteArray()

    fun verifySignature(
        address: String,
        message: ByteArray,
        signature: ByteArray,
        curveName: String = DEFAULT_CURVE,
    ): Boolean =
        verifySignatureApi(address, message.toUByteList(), signature.toUByteList(), curveName)

    fun blake3Hash(data: ByteArray): ByteArray =
        blake3HashApi(data.toUByteList()).toByteArray()

    fun listSupportedCurves(): List<CurveInfoModel> =
        ffiListSupportedCurves().map { it.toModel() }
}

private fun KeyPairData.toModel(): KeyPairModel =
    KeyPairModel(
        privateKey = privateKey,
        publicKey = publicKey,
        address = address,
        taggedAddress = taggedAddress,
        rawPublicKey = rawPublicKey.map { it.toByte() }.toByteArray(),
        curveType = curveType,
    )

private fun CurveInfo.toModel(): CurveInfoModel =
    CurveInfoModel(
        name = name,
        isPostQuantum = isPostQuantum,
        isHybrid = isHybrid,
        securityLevel = securityLevel.toInt(),
    )

private fun ByteArray.toUByteList(): List<UByte> = map { it.toUByte() }

private fun List<UByte>.toByteArray(): ByteArray = map { it.toByte() }.toByteArray()
