package com.yttt.yai

import uniffi.yttt_mobile_core.theme

import androidx.compose.ui.platform.LocalContext

import androidx.compose.foundation.isSystemInDarkTheme

import android.util.Log
import android.view.Choreographer
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import org.connectbot.terminal.Terminal
import org.connectbot.terminal.TerminalEmulatorFactory
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.LinkPath
import uniffi.yttt_mobile_core.MobileCoreException
import uniffi.yttt_mobile_core.TerminalInfo

/** Whether the phone controls a terminal, shared with the screen's top bar for "交还电脑". */
class TerminalControl {
    var controlling by mutableStateOf(false)
    /** Set by the terminal pane while it is on screen. */
    var giveBack: () -> Unit = {}
}

/**
 * Observes at the Host's size (the font shrinks to fit); taking control takes
 * this one terminal at the size the screen fits, and keys go to the Host once
 * it applied that size. Repaints are drained once per display frame. `above`
 * sits between the terminal and the key bar, such as the pane switcher.
 */
@Composable
fun TerminalPane(
    session: HostSession,
    info: TerminalInfo,
    tip: String,
    autoType: String?,
    link: Link,
    control: TerminalControl,
    above: @Composable () -> Unit = {},
) {
    // Laid out at the screen's size while a take is in flight.
    var fitting by remember { mutableStateOf(false) }
    var hostTooOld by remember { mutableStateOf(false) }
    var stats by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val id = info.sessionId
    val controlling = control.controlling
    // The grid termlib fits to the screen, reported once laid out unforced.
    val fitted = remember(id) { MutableStateFlow<Pair<Int, Int>?>(null) }
    // The key bar's one-shot Ctrl applies to the next key typed.
    val ctrl = remember(id) { CtrlKey() }
    var ctrlArmed by remember(id) { mutableStateOf(false) }
    var keyboardShown by remember(id) { mutableStateOf(true) }
    val look by link.appearance.terminal.collectAsState()
    val systemDark = isSystemInDarkTheme()
    val palette = remember(look.theme, systemDark) { theme(look.theme, systemDark) }
    val context = LocalContext.current
    val typeface = remember(look.font) { AppearanceSettings.typeface(context, look.font) }
    val emulator = remember(id) {
        TerminalEmulatorFactory.create(
            initialRows = info.rows.toInt(),
            initialCols = info.cols.toInt(),
            onKeyboardInput = { bytes ->
                if (session.inControl(id)) {
                    val sent = ctrl.apply(bytes)
                    ctrlArmed = ctrl.armed
                    runCatching { session.sendInput(id, sent) }
                }
            },
            onResize = { dimensions ->
                if (dimensions.columns > 0 && dimensions.rows > 0) {
                    fitted.value = dimensions.columns to dimensions.rows
                    if (session.inControl(id)) {
                        scope.launch {
                            runCatching {
                                session.resize(id, dimensions.columns.toUShort(), dimensions.rows.toUShort())
                            }
                        }
                    }
                }
            },
        )
    }
    control.giveBack = {
        control.controlling = false
        scope.launch { runCatching { session.giveBack(listOf(id)) } }
    }

    // Attached while on screen: in the background only the Agent overview
    // stays, so the Host stops sending this terminal's frames.
    val foreground by link.foreground.collectAsState()
    LaunchedEffect(id, foreground) {
        if (foreground) {
            runCatching { session.attach(id) }.onFailure { stats = "attach failed: ${it.message}" }
            link.viewing.value = id
        } else {
            link.viewing.value = null
            runCatching { session.detach(id) }
        }
    }

    DisposableEffect(id) {
        val choreographer = Choreographer.getInstance()
        var frames = 0
        var fed = 0L
        var last = System.nanoTime()
        var running = true
        var pathLabel = "—"
        var ready = true
        var lastOutput = System.nanoTime()
        val callback = object : Choreographer.FrameCallback {
            override fun doFrame(frameTimeNanos: Long) {
                if (!running) return
                frames++
                // The computer took the terminal back, or the connection dropped.
                if (control.controlling && !session.inControl(id)) control.controlling = false
                // Logged with timestamps, to time recovery after a network change.
                val nowReady = session.isReady()
                if (nowReady != ready) {
                    ready = nowReady
                    Log.i(TAG, if (nowReady) "session ready" else "session lost")
                }
                session.drain(id)?.let { bytes ->
                    fed += bytes.size
                    emulator.writeInput(bytes)
                    val gap = (System.nanoTime() - lastOutput) / 1_000_000
                    if (gap > 3000) Log.i(TAG, "output resumed after $gap ms")
                    lastOutput = System.nanoTime()
                }
                val now = System.nanoTime()
                val seconds = (now - last) / 1e9
                if (SHOW_DIAGNOSTICS && seconds >= 0.5) {
                    val drain = session.drainStats()
                    stats = "%s · %.0f fps · %.0f KiB/s · drain p95 %d µs".format(
                        pathLabel, frames / seconds, fed / seconds / 1024, drain.p95Micros.toLong(),
                    )
                    scope.launch {
                        pathLabel = when (session.path()) {
                            LinkPath.DIRECT -> "直连"
                            LinkPath.RELAY -> "经 relay"
                            LinkPath.UNKNOWN -> "—"
                        }
                    }
                    frames = 0
                    fed = 0
                    last = now
                }
                choreographer.postFrameCallback(this)
            }
        }
        choreographer.postFrameCallback(callback)
        onDispose {
            running = false
            link.viewing.value = null
            // Not the screen's scope: it ends with the screen.
            link.scope.launch { runCatching { session.detach(id) } }
        }
    }

    suspend fun takeControl() {
        val hostSize = info.cols.toInt() to info.rows.toInt()
        fitted.value = null
        fitting = true
        val (cols, rows) = withTimeoutOrNull(1000) { fitted.filterNotNull().first() } ?: hostSize
        runCatching { session.takeControl(id, cols.toUShort(), rows.toUShort()) }
            .onSuccess {
                control.controlling = true
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
            }
            .onFailure {
                if (it is MobileCoreException.HostTooOld) hostTooOld = true
                else stats = "接管失败：${it.message}"
            }
        fitting = false
    }

    // termlib clears its grid when the forced size changes; once it has laid
    // out again, ask the core for the whole screen.
    LaunchedEffect(controlling, fitting) {
        withFrameNanos {}
        withFrameNanos {}
        session.repaint(id)
    }

    // The palette's colors, and everything repainted in them.
    LaunchedEffect(emulator, palette) {
        emulator.applyColorScheme(palette.ansi.map(::argb).toIntArray(), argb(palette.foreground), argb(palette.background))
        session.repaint(id)
    }

    LaunchedEffect(autoType) {
        val line = autoType ?: return@LaunchedEffect
        delay(2000)
        takeControl()
        if (control.controlling) runCatching { session.sendInput(id, (line + "\r").toByteArray()) }
    }

    // Above the keyboard while typing, above the navigation bar otherwise.
    Column(
        Modifier
            .fillMaxSize()
            .background(Color(argb(palette.background)))
            .windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars).only(WindowInsetsSides.Bottom)),
    ) {
        Box(Modifier.weight(1f).fillMaxWidth()) {
            Terminal(
                terminalEmulator = emulator,
                typeface = typeface,
                initialFontSize = look.size.sp,
                backgroundColor = Color(argb(palette.background)),
                foregroundColor = Color(argb(palette.foreground)),
                modifier = Modifier.fillMaxSize(),
                keyboardEnabled = controlling && keyboardShown,
                forcedSize = if (controlling || fitting) null else info.rows.toInt() to info.cols.toInt(),
            )
            if (SHOW_DIAGNOSTICS && stats.isNotEmpty()) {
                Text(
                    stats,
                    color = Color.White,
                    fontSize = 10.sp,
                    fontFamily = FontFamily.Monospace,
                    modifier = Modifier.align(Alignment.TopEnd).background(Color(0x99000000)).padding(4.dp),
                )
            }
            if (hostTooOld) {
                Surface(
                    color = MaterialTheme.colorScheme.errorContainer,
                    shape = MaterialTheme.shapes.medium,
                    modifier = Modifier.align(Alignment.BottomCenter).padding(16.dp),
                ) {
                    Text("电脑上的 yttt 版本太旧，不能只接管一个终端。升级后再试。", Modifier.padding(16.dp))
                }
            } else if (!controlling && !fitting) {
                ExtendedFloatingActionButton(
                    onClick = { scope.launch { takeControl() } },
                    text = { Text(tip) },
                    icon = { Icon(Icons.Filled.Keyboard, null) },
                    modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
                )
            }
        }
        above()
        if (controlling) {
            KeyBar(
                ctrlArmed = ctrlArmed,
                keyboardShown = keyboardShown,
                onKey = { key -> runCatching { session.sendKey(id, key) } },
                onCtrl = { ctrl.toggle(); ctrlArmed = ctrl.armed },
                onKeyboard = { keyboardShown = !keyboardShown },
            )
        }
    }
}
