package dev.yttt.spike

import android.app.Application
import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.os.Handler
import android.os.Looper
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.MobileCore
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.OverviewListener

const val TAG = "yttt-spike"

class YtttSpikeApp : Application() {
    val link by lazy { Link(this) }
}

val Context.link: Link get() = (applicationContext as YtttSpikeApp).link

/**
 * The process's one link with the paired Host, shared by the activity (the
 * screens) and the foreground service (notifications), so it outlives the
 * activity going to the background.
 */
class Link(context: Context) {
    private val app = context.applicationContext
    val storage = Storage(app)
    val core = MobileCore()
    val secret: ByteArray = storage.deviceSecret()
    /** Work that must outlive any screen, such as detaching a terminal. */
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    /** Spike automation: keep off direct paths. */
    @Volatile var relayOnly = false

    private val hostStore = HostStore(storage.dir) { core.hostInfo(it).id }
    /** The paired Hosts; the link is with the current one. */
    val hosts = MutableStateFlow(hostStore.load())
    /** The current Host's record. */
    val record = MutableStateFlow(hosts.value.currentRecord)
    val session = MutableStateFlow<HostSession?>(null)
    val overview = MutableStateFlow(Overview(emptyList(), emptyList(), 0u))
    val status = MutableStateFlow("")
    /** The terminal the user looks at in the foreground, if any. */
    val viewing = MutableStateFlow<String?>(null)
    /** Whether the activity is started. */
    val foreground = MutableStateFlow(false)
    /** A terminal to open, from a notification or automation. */
    val openRequest = MutableStateFlow<String?>(null)

    private val connecting = Mutex()
    private val main = Handler(Looper.getMainLooper())
    private val notifyNetwork = Runnable {
        Log.i(TAG, "network changed")
        session.value?.networkChanged()
    }

    /**
     * Tells the core when a network becomes usable, debounced: iroh cannot
     * watch the interfaces itself here (SELinux keeps apps out of netlink and
     * /sys/class/net). Not on losing one: a probe then finds no answer and
     * replaces a connection that would have outlived a short outage.
     * Capability changes are left out; signal strength alone fires them.
     */
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) = changed()
        override fun onLinkPropertiesChanged(network: Network, properties: LinkProperties) = changed()

        private fun changed() {
            main.removeCallbacks(notifyNetwork)
            main.postDelayed(notifyNetwork, 500)
        }
    }

    init {
        app.getSystemService(ConnectivityManager::class.java).registerDefaultNetworkCallback(networkCallback)
    }

    /** Each Host's name, told apart when two share one. */
    fun hostNames(list: HostList): List<Pair<String, String>> =
        list.names { runCatching { core.hostInfo(it).label }.getOrDefault("Host") }

    /** Keeps a newly paired Host (or a Host paired again) and makes it current. */
    suspend fun savePairing(record: ByteArray) = changeHosts(hosts.value.added(core.hostInfo(record).id, record))

    /** Connects to another paired Host instead. */
    suspend fun switchTo(id: String) {
        if (id != hosts.value.current) changeHosts(hosts.value.selected(id))
    }

    /** Forgets the current Host; the link moves to the first one left, if any. */
    suspend fun forget() {
        val current = hosts.value.current ?: return
        changeHosts(hosts.value.removed(current))
    }

    /**
     * Drops the link and what the old Host said (its wait notifications go
     * with its overview), then stores the list; screens connect to the new
     * current Host as `record` changes.
     */
    private suspend fun changeHosts(list: HostList) {
        disconnect()
        overview.value = Overview(emptyList(), emptyList(), 0u)
        viewing.value = null
        hostStore.save(list)
        hosts.value = list
        record.value = list.currentRecord
    }

    /** Connects once, however many screens and services ask at the same time. */
    suspend fun connect(): HostSession? = connecting.withLock {
        session.value?.let { return@withLock it }
        val stored = record.value ?: return@withLock null
        status.value = "正在连接…"
        try {
            val connected = core.connect(stored, secret, relayOnly)
            runCatching { connected.openActiveWorkspace() }
            connected.watchOverview(object : OverviewListener {
                override fun onOverview(overview: Overview) {
                    this@Link.overview.value = overview
                }
            })
            session.value = connected
            status.value = ""
            connected
        } catch (error: Exception) {
            status.value = "连接失败：${error.message}"
            null
        }
    }

    suspend fun disconnect() {
        val current = connecting.withLock { session.value.also { session.value = null } } ?: return
        runCatching { current.disconnect() }
    }
}
