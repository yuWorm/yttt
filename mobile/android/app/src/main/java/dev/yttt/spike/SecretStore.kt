package dev.yttt.spike

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

/**
 * The device secret, encrypted with `key` (AES-GCM) in `device-secret.enc`:
 * the IV, then the ciphertext and tag. The spike's plain `device-secret`
 * moves in once and is deleted once the encrypted copy reads back.
 */
class SecretStore(private val dir: File, private val key: () -> SecretKey) {
    private val encrypted = File(dir, "device-secret.enc")
    private val legacy = File(dir, "device-secret")

    fun deviceSecret(): ByteArray {
        if (encrypted.exists()) return decrypt(encrypted.readBytes())
        val old = legacy.takeIf { it.exists() }?.readBytes()?.takeIf { it.size == 32 }
        val secret = old ?: ByteArray(32).also { SecureRandom().nextBytes(it) }
        write(secret)
        if (old != null && decrypt(encrypted.readBytes()).contentEquals(old)) legacy.delete()
        return secret
    }

    private fun write(secret: ByteArray) {
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val sealed = cipher.iv + cipher.doFinal(secret)
        val partial = File(dir, "device-secret.enc.partial")
        partial.writeBytes(sealed)
        check(partial.renameTo(encrypted)) { "could not store the device secret" }
    }

    private fun decrypt(sealed: ByteArray): ByteArray {
        require(sealed.size > IV_BYTES) { "the stored device secret is damaged" }
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(TAG_BITS, sealed, 0, IV_BYTES))
        return cipher.doFinal(sealed, IV_BYTES, sealed.size - IV_BYTES)
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
