package com.yttt.yai

import android.graphics.BitmapFactory
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
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
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.ContentPaste
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.QrCode2
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.MobileCoreException
import uniffi.yttt_mobile_core.PairingListener

/** Pairing with a Host, from the first launch or "添加 Host": the progress and the digits to compare. */
class Pairing(private val link: Link, private val scope: CoroutineScope) {
    var status by mutableStateOf("")
    var digits by mutableStateOf<UInt?>(null)
    var busy by mutableStateOf(false)

    /** Pairs and makes the Host current; true once it is paired. */
    suspend fun pair(code: String): Boolean {
        busy = true
        status = "正在连接 Host…"
        digits = null
        val listener = object : PairingListener {
            override fun onShortCode(shortCode: UInt) {
                scope.launch {
                    digits = shortCode
                    status = "核对数字，一样就在电脑上确认"
                }
            }
        }
        return try {
            val account = link.account.value?.account
            link.savePairing(link.core.pair(code.trim(), link.deviceName, link.secret, link.relayOnly, account, listener))
            status = ""
            digits = null
            true
        } catch (expired: MobileCoreException.SignInExpired) {
            digits = null
            status = "Relay 账号的登录已过期，请重新登录后再配对。"
            link.signOut(EXPIRED)
            false
        } catch (error: Exception) {
            digits = null
            status = "配对失败：${error.message}"
            false
        } finally {
            busy = false
        }
    }

    fun reset() {
        status = ""
        digits = null
    }
}

/** Scan the computer's QR code, read it from a picture, or paste the code. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairScreen(
    pairing: Pairing,
    account: RelayAccount?,
    accountNotice: String?,
    onSignIn: () -> Unit,
    onBack: (() -> Unit)? = null,
    onPair: (String) -> Unit,
) {
    var code by remember { mutableStateOf("") }
    var scanning by remember { mutableStateOf(false) }
    var scanError by remember { mutableStateOf<String?>(null) }
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    val pickImage = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        val bitmap = context.contentResolver.openInputStream(uri)?.use(BitmapFactory::decodeStream)
        val found = bitmap?.let { decodePairingCode(it.luminance(), it.width, it.height) }
        if (found != null) { scanError = null; onPair(found) } else scanError = "这张图片里没有配对二维码"
    }
    if (scanning) {
        PairingScanner(onCode = { found -> scanning = false; onPair(found) }, onClose = { scanning = false })
        return
    }
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = {
                    if (onBack != null) IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") }
                },
                title = { Text(if (onBack != null) "添加 Host" else "配对 Host") },
            )
        },
    ) { padding ->
        Column(
            Modifier.padding(padding).consumeWindowInsets(padding).fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (pairing.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            AccountStep(account, accountNotice, onSignIn)
            Text("第 2 步 · 配对电脑", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(top = 12.dp))
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.QrCode2, null, Modifier.size(32.dp), tint = MaterialTheme.colorScheme.primary)
                Spacer(Modifier.width(12.dp))
                Text(
                    "在电脑上打开 yttt 的“设置 ▸ 远程访问 ▸ 配对设备”，会显示一个二维码。",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Button(onClick = { scanning = true }, enabled = !pairing.busy, modifier = Modifier.fillMaxWidth()) {
                Icon(Icons.Outlined.QrCodeScanner, null, Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text("扫描二维码")
            }
            OutlinedButton(
                onClick = { pickImage.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
                enabled = !pairing.busy,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Outlined.Image, null, Modifier.size(18.dp))
                Spacer(Modifier.width(8.dp))
                Text("从图片识别")
            }
            scanError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
            Text("或粘贴配对码", style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(top = 12.dp))
            OutlinedTextField(
                value = code,
                onValueChange = { code = it },
                label = { Text("配对码") },
                placeholder = { Text("YTTT-PAIR:1:…") },
                textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
                trailingIcon = {
                    IconButton(onClick = { clipboard.getText()?.text?.let { code = it.trim() } }) {
                        Icon(Icons.Outlined.ContentPaste, "粘贴")
                    }
                },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                maxLines = 4,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                Button(onClick = { onPair(code) }, enabled = code.isNotBlank() && !pairing.busy) { Text("配对") }
            }
            pairing.digits?.let { digits ->
                Card(Modifier.fillMaxWidth()) {
                    Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                        Text("核对数字", style = MaterialTheme.typography.labelLarge)
                        Text("%06d".format(digits.toInt()), style = MaterialTheme.typography.displaySmall,
                            fontFamily = FontFamily.Monospace)
                        Text("和电脑上显示的一样，就在电脑上点“确认”", style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            if (pairing.status.isNotEmpty()) Text(pairing.status)
        }
    }
}
