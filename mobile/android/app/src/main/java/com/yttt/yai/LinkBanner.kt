package com.yttt.yai

import android.os.SystemClock
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.LinkState

/** The banner's words for a link state, and when it offers a fresh connection. */
object LinkBannerText {
    /** Down this long, the banner offers a fresh connection instead of a retry. */
    const val FRESH_AFTER_MILLIS = 15_000L

    fun title(state: LinkState): String = when (state) {
        is LinkState.Ready -> ""
        is LinkState.Connecting -> "正在连接…"
        is LinkState.Reconnecting ->
            if (state.attempt > 1u) "连接断开，正在重连（第 ${state.attempt} 次）…" else "连接断开，正在重连…"
        is LinkState.Lost -> "连接已断开：${state.reason}"
    }
}

/** A thin bar while the link is down: reconnecting with a retry; after a while, or given up, a fresh connection. */
@Composable
fun LinkBanner(link: Link) {
    val state by link.linkState.collectAsState()
    val session by link.session.collectAsState()
    val downSince by link.downSince.collectAsState()
    val scope = rememberCoroutineScope()
    var now by remember { mutableLongStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(downSince) {
        while (downSince != null) {
            now = SystemClock.elapsedRealtime()
            delay(1_000)
        }
    }
    AnimatedVisibility(session != null && state !is LinkState.Ready) {
        val lost = state is LinkState.Lost
        val long = downSince?.let { now - it > LinkBannerText.FRESH_AFTER_MILLIS } ?: false
        Surface(color = MaterialTheme.colorScheme.tertiaryContainer, modifier = Modifier.fillMaxWidth()) {
            Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                if (lost) {
                    Icon(Icons.Outlined.CloudOff, null, Modifier.size(18.dp))
                } else {
                    CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
                }
                Spacer(Modifier.width(12.dp))
                Text(LinkBannerText.title(state), style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f), maxLines = 2)
                if (lost || long) {
                    TextButton(onClick = { scope.launch { link.reconnectFresh() } }) { Text("重新连接") }
                } else {
                    TextButton(onClick = link::retryNow) { Text("立即重试") }
                }
            }
        }
    }
}

/** A terminal opened on its own that the Host no longer runs. */
@androidx.compose.material3.ExperimentalMaterial3Api
@Composable
private fun GoneBar(onBack: () -> Unit) {
    androidx.compose.material3.TopAppBar(
        navigationIcon = {
            androidx.compose.material3.IconButton(onClick = onBack) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, "返回")
            }
        },
        title = { Text("终端") },
    )
}

@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun TerminalGone(onBack: () -> Unit) {
    androidx.compose.foundation.layout.Column(Modifier.fillMaxWidth()) {
        GoneBar(onBack)
        Text(
            "终端已结束：电脑上的这个终端已经不在了。",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(24.dp),
        )
    }
}
