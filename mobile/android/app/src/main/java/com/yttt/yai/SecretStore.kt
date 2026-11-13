package com.yttt.yai

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

private const val KEY_ALIAS = "yttt-device-secret"
private const val IV_BYTES = 12
private const val TAG_BITS = 128

/** Bytes kept encrypted with `key` (AES-GCM) in `dir/name`: the IV, then the ciphertext and tag. */
class SealedFile(private val dir: File, private val name: String, private val key: () -> SecretKey) {
    private val file = File(dir, name)

    val exists: Boolean get() = file.exists()

    fun read(): ByteArray {
        val sealed = file.readBytes()
        require(sealed.size > IV_BYTES) { "$name is damaged" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
        return cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
    }

    fun write(bytes: ByteArray) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        writeDurably(File(dir, "$name.partial"), cipher.iv + cipher.doFinal(bytes), file)
    }

    fun delete() {
        file.delete()
    }
}

/** The device secret, sealed in `device-secret.enc`; made on first use. */
class SecretStore(dir: File, key: () -> SecretKey) {
    private val sealed = SealedFile(dir, "device-secret.enc", key)

    fun deviceSecret(): ByteArray {
        if (sealed.exists) return sealed.read()
        val secret = ByteArray(32).also { SecureRandom().nextBytes(it) }
        sealed.write(secret)
        return secret
    }

    companion object {
        /** The app's AES key in the Android Keystore, made on first use; it never leaves it. */
        fun keystoreKey(): SecretKey {
            val keystore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            (keystore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
            val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
            generator.init(
                KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setKeySize(256)
                    .build(),
            )
            return generator.generateKey()
        }
    }
}
