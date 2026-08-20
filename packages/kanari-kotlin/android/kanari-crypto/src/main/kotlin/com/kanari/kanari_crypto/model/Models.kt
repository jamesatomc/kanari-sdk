package com.kanari.kanari_crypto.model

data class KeyPairModel(
    val privateKey: String,
    val publicKey: String,
    val address: String,
    val taggedAddress: String,
    val rawPublicKey: ByteArray,
    val curveType: String,
) {
    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is KeyPairModel) return false
        return privateKey == other.privateKey &&
            publicKey == other.publicKey &&
            address == other.address &&
            taggedAddress == other.taggedAddress &&
            rawPublicKey.contentEquals(other.rawPublicKey) &&
            curveType == other.curveType
    }

    override fun hashCode(): Int {
        var result = privateKey.hashCode()
        result = 31 * result + publicKey.hashCode()
        result = 31 * result + address.hashCode()
        result = 31 * result + taggedAddress.hashCode()
        result = 31 * result + rawPublicKey.contentHashCode()
        result = 31 * result + curveType.hashCode()
        return result
    }
}

data class CurveInfoModel(
    val name: String,
    val isPostQuantum: Boolean,
    val isHybrid: Boolean,
    val securityLevel: Int,
)
