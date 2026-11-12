package dev.yttt.spike

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.Log
import android.graphics.BitmapFactory
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.compose.material3.OutlinedButton
import androidx.compose.ui.platform.LocalContext
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.DrawerValue
import androidx.compose.material3.ModalNavigationDrawer
import androidx.compose.material3.rememberDrawerState
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.runtime.collectAsState
import androidx.core.content.ContextCompat
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
import uniffi.yttt_mobile_core.LinkPath
import uniffi.yttt_mobile_core.MobileCoreException
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.OverviewListener
import uniffi.yttt_mobile_core.PairingListener
import uniffi.yttt_mobile_core.TerminalInfo
import java.io.File
import java.security.SecureRandom

/** App-private files; the device secret is encrypted with a Keystore key. */
class Storage(context: Context) {
    private val dir = File(context.filesDir, "yttt-spike").apply { mkdirs() }

    fun load(name: String): ByteArray? = File(dir, name).takeIf { it.exists() }?.readBytes()

    fun save(name: String, data: ByteArray?) {
        val file = File(dir, name)
        if (data == null) file.delete() else file.writeBytes(data)
    }

    /** Encrypted with a key that never leaves the Android Keystore. */
    fun deviceSecret(): ByteArray = SecretStore(dir, SecretStore::keystoreKey).deviceSecret()
}

class MainActivity : ComponentActivity() {
    private var autoType: String? = null

    private val askNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { LinkService.start(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Spike-only automation for emulator runs (adb shell am start -e ...).
        // `qrImage`: a picture in the app's files to pair from, read as the
        // image picker reads one.
        val autoPair = intent.getStringExtra("pair")
            ?: intent.getStringExtra("qrImage")?.let { name ->
                BitmapFactory.decodeFile(java.io.File(filesDir, name).path)
                    ?.let { decodePairingCode(it.luminance(), it.width, it.height) }
            }
        autoType = intent.getStringExtra("type")
        link.relayOnly = intent.getBooleanExtra("relayOnly", false)
        takeOpenRequest(intent)
        setContent {
            MaterialTheme {
                Surface(Modifier.fillMaxSize().safeDrawingPadding()) {
                    App(link, autoPair, autoType, onConnected = ::startLinkService, onBattery = ::openBatterySettings)
                }
            }
        }
    }

    /** A notification for a waiting Agent was tapped. */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        takeOpenRequest(intent)
    }

    private fun takeOpenRequest(intent: Intent) {
        intent.getStringExtra("open")?.let { link.openRequest.value = it }
    }

    override fun onStart() {
        super.onStart()
        link.foreground.value = true
    }

    /** Leaving the foreground gives every terminal back to the computer. */
    override fun onStop() {
        super.onStop()
        link.foreground.value = false
        val session = link.session.value ?: return
        link.scope.launch { runCatching { session.giveBack(emptyList()) } }
    }

    /** Keeps the link in the background, asking to notify first (Android 13+). */
    private fun startLinkService() {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            LinkService.start(this)
        }
    }

    private fun openBatterySettings() {
        runCatching { startActivity(Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)) }
    }
}

/** The root page: the overview, a project, or a single terminal (debugging). */
sealed interface Root {
    data object Home : Root
    data class Project(val place: Place) : Root
    data class Terminal(val info: TerminalInfo) : Root
}

@Composable
fun App(
    link: Link,
    autoPair: String?,
    autoType: String?,
    onConnected: () -> Unit,
    onBattery: () -> Unit,
) {
    val record by link.record.collectAsState()
    val session by link.session.collectAsState()
    val overview by link.overview.collectAsState()
    val linkStatus by link.status.collectAsState()
    var terminals by remember { mutableStateOf<List<TerminalInfo>>(emptyList()) }
    var root by remember { mutableStateOf<Root>(Root.Home) }
    // A project file open full screen: (project, path).
    var editing by remember { mutableStateOf<Pair<String, String>?>(null) }
    // Each project's right drawer, kept while a file is open.
    val filesStates = remember { HashMap<String, FilesState>() }
    val filesDrawer = rememberDrawerState(DrawerValue.Closed)
    var status by remember { mutableStateOf("") }
    var pairingDigits by remember { mutableStateOf<UInt?>(null) }
    val scope = rememberCoroutineScope()
    val drawer = rememberDrawerState(DrawerValue.Closed)

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
            link.savePairing(link.core.pair(code.trim(), "Android spike", link.secret, link.relayOnly, listener))
            status = ""
        } catch (error: Exception) {
            status = "配对失败：${error.message}"
        }
    }

    /** Opens the pane running terminal `sessionId`, or the terminal alone. */
    suspend fun openTerminal(sessionId: String) {
        val current = session ?: return
        overview.placeOf(sessionId)?.let {
            root = Root.Project(it)
            return
        }
        if (terminals.none { it.sessionId == sessionId }) runCatching { terminals = current.terminals() }
        val info = terminals.firstOrNull { it.sessionId == sessionId }
        if (info != null) root = Root.Terminal(info) else status = "这个终端已经不在了"
    }

    LaunchedEffect(Unit) {
        if (record == null && autoPair != null) pair(autoPair)
    }
    LaunchedEffect(record) {
        if (record == null) return@LaunchedEffect
        val connected = link.connect() ?: return@LaunchedEffect
        terminals = connected.terminals()
        onConnected()
    }
    // The terminals change as panes start and stop.
    LaunchedEffect(session, overview.terminals) {
        session?.let { current -> runCatching { terminals = current.terminals() } }
    }
    // A tapped notification, or automation, names a terminal to open.
    LaunchedEffect(session, overview.projects.isNotEmpty()) {
        if (session == null) return@LaunchedEffect
        link.openRequest.collect { sessionId ->
            if (sessionId != null) {
                openTerminal(sessionId)
                link.openRequest.value = null
            }
        }
    }
    BackHandler(enabled = editing == null && (root != Root.Home || drawer.isOpen || filesDrawer.isOpen)) {
        when {
            drawer.isOpen -> scope.launch { drawer.close() }
            filesDrawer.isOpen -> scope.launch { filesDrawer.close() }
            else -> root = Root.Home
        }
    }
    val file = editing
    if (file != null && session != null) {
        EditorScreen(session!!, file.first, file.second) { editing = null }
        return
    }

    val current = session
    val hostLabel = record?.let { runCatching { link.core.hostInfo(it).label }.getOrNull() }.orEmpty()
    if (record == null) {
        PairScreen(status, pairingDigits) { code -> scope.launch { pair(code) } }
        return
    }
    ModalNavigationDrawer(
        drawerState = drawer,
        // The terminal takes horizontal drags while observing; the menu button opens it.
        gesturesEnabled = drawer.isOpen,
        drawerContent = {
            LeftDrawer(
                hostLabel = hostLabel,
                overview = overview,
                current = (root as? Root.Project)?.place?.projectId,
                onHome = { root = Root.Home; scope.launch { drawer.close() } },
                onProject = { id -> root = Root.Project(overview.initialPlace(id)); scope.launch { drawer.close() } },
                onForget = { scope.launch { drawer.close(); link.forget(); root = Root.Home } },
            )
        },
    ) {
        when (val page = root) {
            is Root.Project -> if (current != null) {
                // The right drawer: Compose has only a start-side drawer, so it
                // lays out right to left around it and back inside.
                CompositionLocalProvider(LocalLayoutDirection provides LayoutDirection.Rtl) {
                    ModalNavigationDrawer(
                        drawerState = filesDrawer,
                        gesturesEnabled = filesDrawer.isOpen,
                        drawerContent = {
                            CompositionLocalProvider(LocalLayoutDirection provides LayoutDirection.Ltr) {
                                FilesDrawer(
                                    current,
                                    page.place.projectId,
                                    filesStates.getOrPut(page.place.projectId) { FilesState() },
                                ) { path ->
                                    scope.launch { filesDrawer.close() }
                                    editing = page.place.projectId to path
                                }
                            }
                        },
                    ) {
                        CompositionLocalProvider(LocalLayoutDirection provides LayoutDirection.Ltr) {
                            ProjectScreen(
                                session = current,
                                link = link,
                                overview = overview,
                                terminals = terminals,
                                place = page.place,
                                autoType = autoType,
                                onPlace = { root = Root.Project(it) },
                                onMenu = { scope.launch { drawer.open() } },
                                onFiles = { scope.launch { filesDrawer.open() } },
                            )
                        }
                    }
                }
            }
            is Root.Terminal -> if (current != null) {
                TerminalPane(current, page.info, tipText(overview.agents.firstOrNull { it.sessionId == page.info.sessionId }),
                    autoType, link) { root = Root.Home }
            }
            Root.Home -> HomeScreen(
                hostLabel = hostLabel,
                status = status.ifEmpty { linkStatus },
                overview = overview,
                terminals = terminals,
                onAgent = { agent -> scope.launch { openTerminal(agent.sessionId) } },
                onTerminal = { root = Root.Terminal(it) },
                onBattery = onBattery,
                onMenu = { scope.launch { drawer.open() } },
            )
        }
    }
}

@Composable
fun PairScreen(status: String, shortCode: UInt?, onPair: (String) -> Unit) {
    var code by remember { mutableStateOf("") }
    var scanning by remember { mutableStateOf(false) }
    var scanError by remember { mutableStateOf<String?>(null) }
    val context = LocalContext.current
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
    Column(Modifier.fillMaxSize().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("配对 Host", style = MaterialTheme.typography.titleLarge)
        Button(onClick = { scanning = true }, modifier = Modifier.fillMaxWidth()) { Text("扫描电脑上的二维码") }
        OutlinedButton(
            onClick = { pickImage.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly)) },
            modifier = Modifier.fillMaxWidth(),
        ) { Text("从图片识别") }
        scanError?.let { Text(it, color = MaterialTheme.colorScheme.error) }
        Text("或粘贴配对码", style = MaterialTheme.typography.labelLarge)
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
fun TerminalPane(
    session: HostSession,
    info: TerminalInfo,
    tip: String,
    autoType: String?,
    link: Link,
    onBack: (() -> Unit)?,
) {
    var controlling by remember { mutableStateOf(false) }
    // Laid out at the screen's size while a take is in flight.
    var fitting by remember { mutableStateOf(false) }
    var hostTooOld by remember { mutableStateOf(false) }
    var stats by remember { mutableStateOf("") }
    val scope = rememberCoroutineScope()
    val id = info.sessionId
    // The grid termlib fits to the screen, reported once laid out unforced.
    val fitted = remember(id) { MutableStateFlow<Pair<Int, Int>?>(null) }
    // The key bar's one-shot Ctrl applies to the next key typed.
    val ctrl = remember(id) { CtrlKey() }
    var ctrlArmed by remember(id) { mutableStateOf(false) }
    var keyboardShown by remember(id) { mutableStateOf(true) }
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
                if (controlling && !session.inControl(id)) controlling = false
                // Logged with timestamps, to time recovery after a network change.
                val nowReady = session.isReady()
                if (nowReady != ready) {
                    ready = nowReady
                    Log.i(TAG, if (nowReady) "session ready" else "session lost")
                }
                session.drain(id)?.let { bytes ->
                    fed += bytes.size
                    emulator.writeInput(bytes)
                    // The demo ticks every second: a longer gap is an outage.
                    val gap = (System.nanoTime() - lastOutput) / 1_000_000
                    if (gap > 3000) Log.i(TAG, "output resumed after $gap ms")
                    lastOutput = System.nanoTime()
                }
                val now = System.nanoTime()
                val seconds = (now - last) / 1e9
                if (seconds >= 0.5) {
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

    Column(Modifier.fillMaxSize().background(Color.Black).imePadding()) {
    Box(Modifier.weight(1f).fillMaxWidth()) {
        Terminal(
            terminalEmulator = emulator,
            modifier = Modifier.fillMaxSize(),
            keyboardEnabled = controlling && keyboardShown,
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
            if (onBack != null) {
                Text("‹ 返回", color = Color.White, modifier = Modifier.clickable(onClick = onBack).padding(8.dp))
            }
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
                text = { Text(tip) },
                icon = {},
                modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
            )
        }
    }
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
