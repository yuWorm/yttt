package dev.yttt.spike

import android.content.Context
import android.os.Bundle
import android.view.Choreographer
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Button
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import org.connectbot.terminal.Terminal
import org.connectbot.terminal.TerminalEmulatorFactory
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.MobileCore
import uniffi.yttt_mobile_core.MobileCoreException
import uniffi.yttt_mobile_core.PairingListener
import uniffi.yttt_mobile_core.TerminalInfo
import java.io.File
import java.security.SecureRandom

/** Spike storage: app-private files. The real app keeps the secret in the Keystore. */
class Storage(context: Context) {
    private val dir = File(context.filesDir, "yttt-spike").apply { mkdirs() }

    fun load(name: String): ByteArray? = File(dir, name).takeIf { it.exists() }?.readBytes()

    fun save(name: String, data: ByteArray?) {
        val file = File(dir, name)
        if (data == null) file.delete() else file.writeBytes(data)
    }

    fun deviceSecret(): ByteArray =
        load("device-secret")?.takeIf { it.size == 32 }
            ?: ByteArray(32).also { SecureRandom().nextBytes(it); save("device-secret", it) }
}

/** The connected session, so the activity can give terminals back on stop. */
class SessionHolder {
    @Volatile var session: HostSession? = null
}

class MainActivity : ComponentActivity() {
    private val holder = SessionHolder()

    /** Leaving the foreground gives every terminal back to the computer. */
    override fun onStop() {
        super.onStop()
        val session = holder.session ?: return
        lifecycleScope.launch { runCatching { session.giveBack(emptyList()) } }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val storage = Storage(this)
        val core = MobileCore()
        // Spike-only automation for emulator runs (adb shell am start -e ...).
        val autoPair = intent.getStringExtra("pair")
        val autoOpen = intent.getStringExtra("open")
        val autoType = intent.getStringExtra("type")
        setContent {
            MaterialTheme {
                Surface(Modifier.fillMaxSize().safeDrawingPadding()) {
                    App(storage, core, holder, autoPair, autoOpen, autoType)
                }
            }
        }
    }
}

@Composable
fun App(
    storage: Storage,
    core: MobileCore,
    holder: SessionHolder,
    autoPair: String?,
    autoOpen: String?,
    autoType: String?,
) {
    var record by remember { mutableStateOf(storage.load("host.json")) }
    var session by remember { mutableStateOf<HostSession?>(null) }
    var terminals by remember { mutableStateOf<List<TerminalInfo>>(emptyList()) }
    var open by remember { mutableStateOf<TerminalInfo?>(null) }
    var status by remember { mutableStateOf("") }
    var pairingDigits by remember { mutableStateOf<UInt?>(null) }
    val scope = rememberCoroutineScope()
    val secret = remember { storage.deviceSecret() }

    suspend fun pair(code: String) {
        status = "正在连接 Host…"
        val listener = object : PairingListener {
            override fun onShortCode(shortCode: UInt) {
                scope.launch {
                    pairingDigits = shortCode
                    status = "在电脑上核对数字并确认"
                }
            }
        }
        try {
            val bytes = core.pair(code.trim(), "Android spike", secret, listener)
            storage.save("host.json", bytes)
            record = bytes
            status = ""
        } catch (error: Exception) {
            status = "配对失败：${error.message}"
        }
    }

    LaunchedEffect(Unit) {
        if (record == null && autoPair != null) pair(autoPair)
    }
    LaunchedEffect(record) {
        val stored = record ?: return@LaunchedEffect
        status = "正在连接…"
        try {
            val connected = core.connect(stored, secret)
            session = connected
            holder.session = connected
            terminals = connected.terminals()
            status = ""
            if (autoOpen != null) open = terminals.firstOrNull { it.sessionId == autoOpen }
        } catch (error: Exception) {
            status = "连接失败：${error.message}"
        }
    }

    val current = session
    val terminal = open
    when {
        record == null -> PairScreen(status, pairingDigits) { code -> scope.launch { pair(code) } }
        current != null && terminal != null ->
            TerminalScreen(current, terminal, autoType) { open = null }
        else -> Column(Modifier.fillMaxSize()) {
            Text(
                runCatching { core.hostLabel(record!!) }.getOrDefault(""),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(16.dp),
            )
            if (status.isNotEmpty()) Text(status, Modifier.padding(horizontal = 16.dp))
            LazyColumn {
                items(terminals, key = { it.sessionId }) { info ->
                    ListItem(
                        headlineContent = { Text(info.sessionId, fontFamily = FontFamily.Monospace) },
                        supportingContent = { Text("${info.cols}×${info.rows} · ${info.projectId}") },
                        modifier = Modifier.clickable { open = info },
                    )
                }
            }
        }
    }
}

@Composable
fun PairScreen(status: String, shortCode: UInt?, onPair: (String) -> Unit) {
    var code by remember { mutableStateOf("") }
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("配对 Host", style = MaterialTheme.typography.titleLarge)
        OutlinedTextField(
            value = code,
            onValueChange = { code = it },
            label = { Text("YTTT-PAIR:1:…") },
            modifier = Modifier.fillMaxWidth(),
        )
        Button(onClick = { onPair(code) }, enabled = code.isNotBlank()) { Text("配对") }
        shortCode?.let { Text("%06d".format(it.toInt()), style = MaterialTheme.typography.displaySmall) }
        if (status.isNotEmpty()) Text(status)
    }
}

/**
 * Observes at the Host's size (the font shrinks to fit); taking control takes
 * this one terminal at the size the screen fits, and keys go to the Host once
 * it applied that size. Repaints are drained once per display frame.
 */
@Composable
fun TerminalScreen(session: HostSession, info: TerminalInfo, autoType: String?, onBack: () -> Unit) {
    var controlling by remember { mutableStateOf(false) }
    // Laid out at the screen's size while a take is in flight.
    var fitting by remember { mutableStateOf(false) }
    var hostTooOld by remember { mutableStateOf(false) }
    var stats by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val id = info.sessionId
    // The grid termlib fits to the screen, reported once laid out unforced.
    val fitted = remember(id) { MutableStateFlow<Pair<Int, Int>?>(null) }
    val emulator = remember(id) {
        TerminalEmulatorFactory.create(
            initialRows = info.rows.toInt(),
            initialCols = info.cols.toInt(),
            onKeyboardInput = { bytes ->
                if (session.inControl(id)) runCatching { session.sendInput(id, bytes) }
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

    DisposableEffect(id) {
        scope.launch { runCatching { session.attach(id) }.onFailure { stats = "attach failed: ${it.message}" } }
        val choreographer = Choreographer.getInstance()
        var frames = 0
        var fed = 0L
        var last = System.nanoTime()
        var running = true
        val callback = object : Choreographer.FrameCallback {
            override fun doFrame(frameTimeNanos: Long) {
                if (!running) return
                frames++
                // The computer took the terminal back, or the connection dropped.
                if (controlling && !session.inControl(id)) controlling = false
                session.drain(id)?.let { bytes ->
                    fed += bytes.size
                    emulator.writeInput(bytes)
                }
                val now = System.nanoTime()
                val seconds = (now - last) / 1e9
                if (seconds >= 0.5) {
                    val drain = session.drainStats()
                    stats = "%.0f fps · %.0f KiB/s · drain p95 %d µs max %d µs".format(
                        frames / seconds, fed / seconds / 1024, drain.p95Micros.toLong(), drain.maxMicros.toLong(),
                    )
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
            scope.launch { runCatching { session.detach(id) } }
        }
    }

    suspend fun takeControl() {
        val hostSize = info.cols.toInt() to info.rows.toInt()
        fitted.value = null
        fitting = true
        val (cols, rows) = withTimeoutOrNull(1000) { fitted.filterNotNull().first() } ?: hostSize
        runCatching { session.takeControl(id, cols.toUShort(), rows.toUShort()) }
            .onSuccess { controlling = true }
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

    LaunchedEffect(autoType) {
        val line = autoType ?: return@LaunchedEffect
        delay(2000)
        takeControl()
        if (controlling) runCatching { session.sendInput(id, (line + "\r").toByteArray()) }
    }

    Box(Modifier.fillMaxSize().background(Color.Black).imePadding()) {
        Terminal(
            terminalEmulator = emulator,
            modifier = Modifier.fillMaxSize(),
            keyboardEnabled = controlling,
            forcedSize = if (controlling || fitting) null else info.rows.toInt() to info.cols.toInt(),
        )
        Text(
            stats,
            color = Color.White,
            fontSize = 10.sp,
            fontFamily = FontFamily.Monospace,
            modifier = Modifier.align(Alignment.TopEnd).background(Color(0x99000000)).padding(4.dp),
        )
        Row(Modifier.align(Alignment.TopStart).padding(4.dp)) {
            Text("‹ 返回", color = Color.White, modifier = Modifier.clickable(onClick = onBack).padding(8.dp))
            if (controlling) {
                Text(
                    "交还电脑",
                    color = Color.White,
                    modifier = Modifier
                        .clickable {
                            controlling = false
                            scope.launch { runCatching { session.giveBack(listOf(id)) } }
                        }
                        .padding(8.dp),
                )
            }
        }
        if (hostTooOld) {
            Text(
                "电脑上的 yttt 版本太旧，不能只接管一个终端。升级后再试。",
                color = Color.White,
                modifier = Modifier.align(Alignment.BottomCenter).background(Color(0xCC000000)).padding(16.dp),
            )
        } else if (!controlling && !fitting) {
            ExtendedFloatingActionButton(
                onClick = { scope.launch { takeControl() } },
                text = { Text("接管终端") },
                icon = {},
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
            )
        }
    }
}
