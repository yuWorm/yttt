package com.yttt.yai

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.outlined.AccountCircle
import androidx.compose.material.icons.outlined.OpenInBrowser
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.Verified
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.BrowserSignIn
import uniffi.yttt_mobile_core.MobileCoreException
import uniffi.yttt_mobile_core.SignedIn

private const val PREFS = "yttt"
private const val CONTROL_URL = "relayControlUrl"

/**
 * Signing in to the relay account: in the browser, where a password manager
 * fills in the console's page, or with the email and password here. It lives
 * in the link's scope, so the sign-in outlives the activity stopping while
 * the browser is in front.
 */
class SignIn(private val link: Link) {
    var busy by mutableStateOf(false)
    var waitingForBrowser by mutableStateOf(false)
    var error by mutableStateOf<String?>(null)
    private var pending: BrowserSignIn? = null

    /** Opens the console's page in a Custom Tab; true once signed in. */
    suspend fun inBrowser(context: Context, controlUrl: String): Boolean = run {
        try {
            val signIn = link.core.beginBrowserSignIn(controlUrl, link.deviceName)
            pending = signIn
            waitingForBrowser = true
            SignInService.start(context)
            CustomTabsIntent.Builder().setShowTitle(true).build().launchUrl(context, Uri.parse(signIn.authorizeUrl()))
            val signedIn = signIn.finish()
            // Back from the browser: the tab sits above the activity in its task.
            context.startActivity(
                Intent(context, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            )
            done(context, signedIn)
        } finally {
            SignInService.stop(context)
            pending = null
            waitingForBrowser = false
        }
    }

    suspend fun withPassword(context: Context, controlUrl: String, email: String, password: String): Boolean = run {
        done(context, link.core.signInPassword(controlUrl, email.trim(), password))
    }

    /** Stops waiting for the browser, as when the user gives up on it. */
    fun cancel() {
        pending?.cancel()
    }

    private suspend fun run(work: suspend () -> Unit): Boolean {
        busy = true
        error = null
        return try {
            work()
            true
        } catch (cancelled: MobileCoreException.Cancelled) {
            false
        } catch (failure: Exception) {
            error = "登录失败：${failure.message}"
            false
        } finally {
            busy = false
        }
    }

    private suspend fun done(context: Context, signedIn: SignedIn) {
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putString(CONTROL_URL, signedIn.controlUrl).apply()
        link.signIn(signedIn)
    }
}

private fun validUrl(text: String): Boolean {
    val uri = runCatching { Uri.parse(text.trim()) }.getOrNull() ?: return false
    return (uri.scheme == "https" || uri.scheme == "http") && !uri.host.isNullOrEmpty()
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SignInScreen(signIn: SignIn, onBack: () -> Unit, onSignIn: (suspend () -> Boolean) -> Unit) {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(PREFS, Context.MODE_PRIVATE) }
    var controlUrl by remember { mutableStateOf(prefs.getString(CONTROL_URL, null) ?: "https://") }
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    val valid = validUrl(controlUrl)
    // Leaving the page gives up on a browser sign-in still under way.
    DisposableEffect(Unit) { onDispose { signIn.cancel() } }

    fun withPassword() {
        if (!signIn.busy && valid && email.isNotBlank() && password.isNotEmpty()) {
            onSignIn { signIn.withPassword(context, controlUrl.trim(), email, password) }
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
                title = { Text("登录 Relay 账号") },
            )
        },
    ) { padding ->
        Column(
            Modifier.padding(padding).consumeWindowInsets(padding).fillMaxSize().imePadding()
                .verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OutlinedTextField(
                value = controlUrl,
                onValueChange = { controlUrl = it },
                label = { Text("控制台地址") },
                placeholder = { Text("https://control.example.com") },
                supportingText = { Text("和电脑上“设置 ▸ 远程访问 ▸ Relay 账号”里的地址相同。") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
                modifier = Modifier.fillMaxWidth(),
            )
            if (signIn.waitingForBrowser) {
                Card(Modifier.fillMaxWidth()) {
                    Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                        Spacer(Modifier.width(12.dp))
                        Text("在浏览器里登录并允许，完成后会回到这里。", Modifier.weight(1f))
                        TextButton(onClick = signIn::cancel) { Text("取消") }
                    }
                }
            } else {
                Button(
                    onClick = { onSignIn { signIn.inBrowser(context, controlUrl.trim()) } },
                    enabled = valid && !signIn.busy,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Outlined.OpenInBrowser, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text("在浏览器中登录")
                }
                Text(
                    "在网页上登录并允许，可以用密码管理器自动填充。",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Text("或用邮箱和密码", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
            OutlinedTextField(
                value = email,
                onValueChange = { email = it },
                label = { Text("邮箱") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Next),
                modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username + ContentType.EmailAddress },
            )
            OutlinedTextField(
                value = password,
                onValueChange = { password = it },
                label = { Text("密码") },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { withPassword() }),
                modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Password },
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End, verticalAlignment = Alignment.CenterVertically) {
                if (signIn.busy && !signIn.waitingForBrowser) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(12.dp))
                }
                OutlinedButton(
                    onClick = ::withPassword,
                    enabled = !signIn.busy && valid && email.isNotBlank() && password.isNotEmpty(),
                ) { Text("登录") }
            }
            signIn.error?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        }
    }
}

/** The pairing page's first step: sign in, so pairing goes through the relay. */
@Composable
fun AccountStep(account: RelayAccount?, notice: String?, onSignIn: () -> Unit) {
    Text("第 1 步 · 账号", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary)
    if (account != null) {
        Card(Modifier.fillMaxWidth()) {
            Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.Verified, null, tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(12.dp))
                Column {
                    Text("已登录 Relay 账号")
                    Text(account.email, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
    } else {
        Card(
            Modifier.fillMaxWidth().clickable(onClick = onSignIn),
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer),
        ) {
            Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.PersonAdd, null)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text("先登录 Relay 账号")
                    Text("登录后，手机在外面也能连上电脑。", style = MaterialTheme.typography.bodySmall)
                }
                Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null)
            }
        }
        Text(
            notice ?: "只在同一个网络里用，或用自建的 Relay，可以不登录。",
            style = MaterialTheme.typography.bodySmall,
            color = if (notice != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** The account icon for the settings page. */
val AccountIcon = Icons.Outlined.AccountCircle
