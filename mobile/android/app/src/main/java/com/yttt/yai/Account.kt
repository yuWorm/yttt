package com.yttt.yai

import org.json.JSONObject
import java.io.File
import java.net.URI
import java.util.Base64
import javax.crypto.SecretKey

/**
 * The signed-in relay account, as the core returned it: `account` goes back
 * to the core to pair and connect; the rest is for the settings page.
 */
class RelayAccount(val account: ByteArray, val email: String, val controlUrl: String) {
    /** The control plane's host, for a subtitle. */
    val controlHost: String get() = runCatching { URI(controlUrl).host }.getOrNull() ?: controlUrl
}

/** The account, sealed with the Keystore key in `relay-account.enc`; a damaged file is no account. */
class AccountStore(dir: File, key: () -> SecretKey) {
    private val sealed = SealedFile(dir, "relay-account.enc", key)

    fun load(): RelayAccount? = runCatching {
        if (!sealed.exists) return null
        val json = JSONObject(String(sealed.read(), Charsets.UTF_8))
        RelayAccount(
            account = Base64.getDecoder().decode(json.getString("account")),
            email = json.getString("email"),
            controlUrl = json.getString("controlUrl"),
        )
    }.getOrNull()

    fun save(account: RelayAccount) {
        val json = JSONObject()
            .put("account", Base64.getEncoder().encodeToString(account.account))
            .put("email", account.email)
            .put("controlUrl", account.controlUrl)
        sealed.write(json.toString().toByteArray(Charsets.UTF_8))
    }

    fun delete() = sealed.delete()
}
