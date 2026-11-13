package com.yttt.yai

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.BitmapFactory
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.ui.Modifier
import androidx.core.content.ContextCompat
import kotlinx.coroutines.launch
import java.io.File

/** App-private files: the paired Hosts (`HostStore`), the device secret encrypted with a Keystore key. */
class Storage(context: Context) {
    val dir = File(context.filesDir, "yttt").apply { mkdirs() }

    /** Encrypted with a key that never leaves the Android Keystore. */
    fun deviceSecret(): ByteArray = SecretStore(dir, SecretStore::keystoreKey).deviceSecret()

    /** The relay account, sealed with the same Keystore key. */
    fun accountStore() = AccountStore(dir, SecretStore::keystoreKey)
}

class MainActivity : ComponentActivity() {
    private var autoType: String? = null

    private val askNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { LinkService.start(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // Debug builds only (see `launchHooks`).
        val hooks = launchHooks(
            extra = intent::getStringExtra,
            flag = { intent.getBooleanExtra(it, false) },
            decodeImage = { name ->
                BitmapFactory.decodeFile(java.io.File(filesDir, name).path)
                    ?.let { decodePairingCode(it.luminance(), it.width, it.height) }
            },
        )
        // Pair once per launch, not again on recreation.
        val autoPair = if (savedInstanceState != null) null else hooks.pair
        autoType = hooks.type
        link.relayOnly = hooks.relayOnly
        takeOpenRequest(intent)
        setContent {
            YtttTheme {
                Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
                    App(
                        link, autoPair, autoType,
                        onConnected = ::startLinkService,
                        onBattery = ::openBatterySettings,
                        onNotifications = ::openNotificationSettings,
                    )
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

    private fun openNotificationSettings() {
        runCatching {
            startActivity(Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, packageName))
        }
    }
}
