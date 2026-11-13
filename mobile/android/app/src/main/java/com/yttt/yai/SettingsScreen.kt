package com.yttt.yai

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.ListAlt
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.outlined.Code
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.BatterySaver
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.LaptopMac
import androidx.compose.material.icons.outlined.LinkOff
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import uniffi.yttt_mobile_core.AccountDetails

/** Settings: the paired Hosts, the system settings the link depends on, and about. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    hosts: List<Pair<String, String>>,
    current: String?,
    connected: Boolean,
    onSwitch: (String) -> Unit,
    onForget: (String) -> Unit,
    editableName: (String) -> String,
    onRename: (String, String) -> Unit,
    onAdd: () -> Unit,
    onBattery: () -> Unit,
    onNotifications: () -> Unit,
    onLicenses: () -> Unit,
    onLog: () -> Unit,
    account: RelayAccount?,
    accountNotice: String?,
    viaAccount: Boolean,
    details: AccountDetails?,
    onSignIn: () -> Unit,
    onSignOut: () -> Unit,
    terminalTheme: String,
    editorTheme: String,
    onTerminalLook: () -> Unit,
    onEditorLook: () -> Unit,
) {
    val context = LocalContext.current
    val version = remember {
        runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: ""
    }
    var forgetting by remember { mutableStateOf<Pair<String, String>?>(null) }
    var signingOut by remember { mutableStateOf(false) }
    var renaming by remember { mutableStateOf<String?>(null) }
    var newName by remember { mutableStateOf("") }
    Scaffold(contentWindowInsets = WindowInsets(0), topBar = { TopAppBar(title = { Text("设置") }) }) { padding ->
        LazyColumn(Modifier.padding(padding).fillMaxWidth()) {
            item { Header("Host") }
            items(hosts, key = { it.first }) { (id, name) ->
                var menu by remember { mutableStateOf(false) }
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.LaptopMac, null) },
                    headlineContent = { Text(name) },
                    supportingContent = if (id == current) {
                        { Text(if (connected) "已连接 · 当前" else "当前") }
                    } else null,
                    trailingContent = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (id == current) Icon(Icons.Filled.CheckCircle, "当前", tint = MaterialTheme.colorScheme.primary)
                            Box {
                                IconButton(onClick = { menu = true }) { Icon(Icons.Filled.MoreVert, "更多") }
                                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                                    DropdownMenuItem(
                                        leadingIcon = { Icon(Icons.Outlined.Edit, null) },
                                        text = { Text("重命名") },
                                        onClick = { menu = false; newName = editableName(id); renaming = id },
                                    )
                                    DropdownMenuItem(
                                        leadingIcon = { Icon(Icons.Outlined.LinkOff, null) },
                                        text = { Text("忘记") },
                                        onClick = { menu = false; forgetting = id to name },
                                    )
                                }
                            }
                        }
                    },
                    modifier = Modifier.clickable { onSwitch(id) },
                )
            }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Add, null) },
                    headlineContent = { Text("添加 Host") },
                    modifier = Modifier.clickable(onClick = onAdd),
                )
            }
            item { Header("外观") }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Terminal, null) },
                    headlineContent = { Text("终端") },
                    supportingContent = { Text("字体、字号、${themeLabel(terminalTheme)}") },
                    modifier = Modifier.clickable(onClick = onTerminalLook),
                )
            }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Code, null) },
                    headlineContent = { Text("编辑器") },
                    supportingContent = { Text("语法高亮、字体、${themeLabel(editorTheme)}") },
                    modifier = Modifier.clickable(onClick = onEditorLook),
                )
            }
            item { Header("Relay 账号") }
            if (account != null) {
                item {
                    ListItem(
                        leadingContent = { Icon(AccountIcon, null) },
                        headlineContent = { Text(account.email) },
                        supportingContent = { Text(account.controlHost) },
                    )
                }
                details?.let { details ->
                    item {
                        ListItem(
                            leadingContent = { Icon(Icons.Outlined.Devices, null) },
                            headlineContent = { Text("套餐 ${details.plan}") },
                            supportingContent = {
                                Text(
                                    if (details.suspended) "账号已暂停，Relay 暂时不能用"
                                    else "设备 ${details.devices} / ${details.maxDevices}",
                                    color = if (details.suspended) MaterialTheme.colorScheme.error else Color.Unspecified,
                                )
                            },
                        )
                    }
                }
                item {
                    ListItem(
                        leadingContent = { Icon(Icons.AutoMirrored.Outlined.Logout, null, tint = MaterialTheme.colorScheme.error) },
                        headlineContent = { Text("退出登录", color = MaterialTheme.colorScheme.error) },
                        modifier = Modifier.clickable { signingOut = true },
                    )
                }
            } else {
                item {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.PersonAdd, null) },
                        headlineContent = { Text("登录 Relay 账号") },
                        modifier = Modifier.clickable(onClick = onSignIn),
                    )
                }
            }
            item {
                Text(
                    when {
                        accountNotice != null -> accountNotice
                        account == null -> "登录后手机经你的 Relay 连接电脑，不在同一个网络也能用。"
                        connected && !viaAccount -> "没能从账号拿到 Relay 凭证，这次只能直连。"
                        else -> "手机经这个账号的 Relay 连接电脑。"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = if (accountNotice != null) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
            }
            item { Header("后台") }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.BatterySaver, null) },
                    headlineContent = { Text("后台运行") },
                    supportingContent = { Text("关闭电池优化，后台不容易断开") },
                    trailingContent = { Icon(Icons.AutoMirrored.Outlined.OpenInNew, null) },
                    modifier = Modifier.clickable(onClick = onBattery),
                )
            }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.NotificationsActive, null) },
                    headlineContent = { Text("通知") },
                    supportingContent = { Text("Agent 等待时提醒") },
                    trailingContent = { Icon(Icons.AutoMirrored.Outlined.OpenInNew, null) },
                    modifier = Modifier.clickable(onClick = onNotifications),
                )
            }
            item { Header("关于") }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Info, null) },
                    headlineContent = { Text("版本") },
                    supportingContent = { Text(version) },
                )
            }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.AutoMirrored.Outlined.ListAlt, null) },
                    headlineContent = { Text("连接日志") },
                    supportingContent = { Text("连接慢或失败时，看看卡在哪一步") },
                    modifier = Modifier.clickable(onClick = onLog),
                )
            }
            item {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Description, null) },
                    headlineContent = { Text("开源许可") },
                    modifier = Modifier.clickable(onClick = onLicenses),
                )
            }
        }
    }
    renaming?.let { id ->
        AlertDialog(
            onDismissRequest = { renaming = null },
            icon = { Icon(Icons.Outlined.Edit, null) },
            title = { Text("重命名 Host") },
            text = {
                Column {
                    OutlinedTextField(
                        value = newName,
                        onValueChange = { newName = it },
                        label = { Text("名称") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        "只改这台手机上显示的名称。留空则用电脑自己的名称。",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            },
            confirmButton = { TextButton(onClick = { renaming = null; onRename(id, newName) }) { Text("好") } },
            dismissButton = { TextButton(onClick = { renaming = null }) { Text("取消") } },
        )
    }
    if (signingOut) {
        AlertDialog(
            onDismissRequest = { signingOut = false },
            icon = { Icon(Icons.AutoMirrored.Outlined.Logout, null) },
            title = { Text("退出 Relay 账号？") },
            text = { Text("退出后，手机只能在同一个网络里直连电脑，或经自建的 Relay。") },
            confirmButton = {
                TextButton(onClick = { signingOut = false; onSignOut() }) {
                    Text("退出登录", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { signingOut = false }) { Text("取消") } },
        )
    }
    forgetting?.let { (id, name) ->
        AlertDialog(
            onDismissRequest = { forgetting = null },
            icon = { Icon(Icons.Outlined.LinkOff, null) },
            title = { Text("忘记 $name？") },
            text = { Text("这台手机不再连接它。要再用，需要在电脑上重新配对。") },
            confirmButton = {
                TextButton(onClick = { forgetting = null; onForget(id) }) {
                    Text("忘记", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = { TextButton(onClick = { forgetting = null }) { Text("取消") } },
        )
    }
}

@Composable
private fun Header(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, top = 20.dp, bottom = 4.dp),
    )
}

private val LICENSES = listOf(
    "sora-editor（含 language-textmate）" to "LGPL-2.1，未修改，作为独立库使用",
    "VS Code 语法文件（TextMate）" to "MIT",
    "JetBrains Mono" to "SIL OFL 1.1",
    "ConnectBot termlib" to "Apache-2.0",
    "JNA" to "Apache-2.0 / LGPL-2.1",
    "ZXing" to "Apache-2.0",
    "Jetpack Compose、AndroidX、CameraX" to "Apache-2.0",
    "Material Icons" to "Apache-2.0",
    "Lobe Icons（Agent 图标）" to "MIT",
    "Oh My Pi（图标）" to "MIT",
)

/** The open-source libraries the app ships with, and their licenses. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LicensesScreen(onBack: () -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
                title = { Text("开源许可") },
            )
        },
    ) { padding ->
        LazyColumn(Modifier.padding(padding)) {
            items(LICENSES) { (name, license) ->
                ListItem(headlineContent = { Text(name) }, supportingContent = { Text(license) })
            }
        }
    }
}
