package com.yttt.yai

import android.app.Application
import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.yttt_mobile_core.HostSession
import uniffi.yttt_mobile_core.MobileCore
import uniffi.yttt_mobile_core.MobileCoreException
import uniffi.yttt_mobile_core.RelaySource
import uniffi.yttt_mobile_core.SignedIn
import uniffi.yttt_mobile_core.Overview
import uniffi.yttt_mobile_core.OverviewListener

const val TAG = "yttt"

class YtttApp : Application() {
    val link by lazy { Link(this) }
}

val Context.link: Link get() = (applicationContext as YtttApp).link

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

    /** Debug automation: keep off direct paths. */
    @Volatile var relayOnly = false

    /** What the Host lists this phone as: its name in Settings, else the model. */
    val deviceName: String =
        Settings.Global.getString(app.contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
            ?: Build.MODEL

    private val hostStore = HostStore(storage.dir) { Log.w(TAG, "the paired Hosts could not be read", it) }
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

    private val accountStore = storage.accountStore()
    /** The signed-in relay account, if any: pairing and connecting go through the Host's relay node with its tickets. */
    val account = MutableStateFlow(accountStore.load())
    /** Why the account was signed out, until the user signs in again. */
    val accountNotice = MutableStateFlow<String?>(null)
    /** Where the current link's relay came from. */
    val relaySource = MutableStateFlow(RelaySource.NONE)

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

    /** Keeps a new sign-in and reconnects through its relay. */
    suspend fun signIn(signedIn: SignedIn) {
        val signed = RelayAccount(signedIn.account, signedIn.email, signedIn.controlUrl)
        accountStore.save(signed)
        account.value = signed
        accountNotice.value = null
        reconnect()
    }

    /** Forgets the account; `notice` says why when the phone did it itself. */
    suspend fun signOut(notice: String? = null) {
        forgetAccount(notice)
        reconnect()
    }

    private fun forgetAccount(notice: String?) {
        accountStore.delete()
        account.value = null
        accountNotice.value = notice
    }

    /** Connects to the current Host again, as after the account changed. */
    private suspend fun reconnect() {
        if (record.value == null) return
        disconnect()
        connect()
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

    /**
     * Forgets a Host. Forgetting the current one moves the link to the first
     * one left, if any; another one just leaves the list.
     */
    suspend fun forget(id: String) {
        // Gives the key's device slot on the account back; the Host is forgotten here either way.
        val signed = account.value
        val stored = hosts.value.hosts.firstOrNull { it.id == id }?.record
        if (signed != null && stored != null) runCatching { core.releaseHost(signed.account, stored, secret) }
        val list = hosts.value.removed(id)
        if (id == hosts.value.current) {
            changeHosts(list)
        } else {
            hostStore.save(list)
            hosts.value = list
        }
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
            val connected = try {
                core.connect(stored, secret, relayOnly, account.value?.account, deviceName)
            } catch (expired: MobileCoreException.SignInExpired) {
                // Without the account: only direct paths or a relay that admits anyone.
                forgetAccount(EXPIRED)
                core.connect(stored, secret, relayOnly, null, deviceName)
            }
            relaySource.value = connected.relaySource()
            runCatching { connected.openActiveWorkspace() }
            // Current before the first overview arrives, which the listener checks.
            session.value = connected
            status.value = ""
            connected.watchOverview(object : OverviewListener {
                override fun onOverview(overview: Overview) {
                    // A link dropped since (a Host change) may still deliver one.
                    if (session.value === connected) this@Link.overview.value = overview
                }
            })
            connected
        } catch (error: Exception) {
            status.value = "连接失败：${error.message}"
            null
        }
    }

    suspend fun disconnect() {
        val current = connecting.withLock { session.value.also { session.value = null } } ?: return
        relaySource.value = RelaySource.NONE
        runCatching { current.disconnect() }
    }
}

const val EXPIRED = "Relay 账号的登录已过期，请重新登录。"
