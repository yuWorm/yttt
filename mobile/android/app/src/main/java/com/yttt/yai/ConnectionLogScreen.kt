package com.yttt.yai

import android.content.Intent
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import java.io.File
import uniffi.yttt_mobile_core.readConnectionLog

/** The core's connection log, newest at the bottom, to read or share when connecting was slow or failed. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConnectionLogScreen(dir: File, onBack: () -> Unit) {
    val context = LocalContext.current
    var text by remember { mutableStateOf("") }
    val scroll = rememberScrollState()
    fun load() { text = readConnectionLog(dir.path) }
    LaunchedEffect(Unit) { load() }
    LaunchedEffect(text) { scroll.scrollTo(scroll.maxValue) }
    Scaffold(
        topBar = {
            TopAppBar(
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回") } },
                title = { Text("连接日志") },
                actions = {
                    IconButton(onClick = ::load) { Icon(Icons.Outlined.Refresh, "刷新") }
                    IconButton(onClick = {
                        val send = Intent(Intent.ACTION_SEND).setType("text/plain")
                            .putExtra(Intent.EXTRA_SUBJECT, "yttt 连接日志").putExtra(Intent.EXTRA_TEXT, text)
                        context.startActivity(Intent.createChooser(send, "分享连接日志"))
                    }, enabled = text.isNotEmpty()) { Icon(Icons.Outlined.Share, "分享") }
                },
            )
        },
    ) { padding ->
        SelectionContainer(Modifier.padding(padding).fillMaxSize().verticalScroll(scroll).horizontalScroll(rememberScrollState())) {
            Text(
                text.ifEmpty { "还没有记录。" },
                fontFamily = FontFamily.Monospace,
                fontSize = 10.sp,
                softWrap = false,
                modifier = Modifier.padding(12.dp),
            )
        }
    }
}
