package com.yttt.yai

import android.os.Bundle
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Dashboard
import androidx.compose.material.icons.filled.Folder
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.outlined.Dashboard
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import kotlinx.coroutines.launch
import uniffi.yttt_mobile_core.TerminalInfo

private const val OVERVIEW = "overview"
private const val PROJECTS = "projects"
private const val SETTINGS = "settings"
private const val PAIR = "pair"
private const val LICENSES = "licenses"

private class TopLevel(val route: String, val label: String, val selected: ImageVector, val unselected: ImageVector)

private val TOP_LEVEL = listOf(
    TopLevel(OVERVIEW, "总控", Icons.Filled.Dashboard, Icons.Outlined.Dashboard),
    TopLevel(PROJECTS, "项目", Icons.Filled.Folder, Icons.Outlined.Folder),
    TopLevel(SETTINGS, "设置", Icons.Filled.Settings, Icons.Outlined.Settings),
)

private fun Bundle?.toArguments(vararg keys: String): Map<String, String?> = keys.associateWith { this?.getString(it) }

private fun optional(name: String) = navArgument(name) { type = NavType.StringType; nullable = true; defaultValue = null }

/** Shows a top-level destination, keeping each one's own back stack. */
private fun NavHostController.showTopLevel(route: String) = navigate(route) {
    popUpTo(graph.findStartDestination().id) { saveState = true }
    launchSingleTop = true
    restoreState = true
}

/**
 * The app once a Host is paired: 总控, 项目 and 设置 in the navigation bar,
 * projects, terminals and files pushed over them; before that, pairing.
 */
@Composable
fun App(
    link: Link,
    autoPair: String?,
    autoType: String?,
    onConnected: () -> Unit,
    onBattery: () -> Unit,
    onNotifications: () -> Unit,
) {
    val record by link.record.collectAsState()
    val hosts by link.hosts.collectAsState()
    val session by link.session.collectAsState()
    val overview by link.overview.collectAsState()
    val linkStatus by link.status.collectAsState()
    var terminals by remember { mutableStateOf<List<TerminalInfo>>(emptyList()) }
    val scope = rememberCoroutineScope()
    val pairing = remember { Pairing(link, scope) }
    // Each project's files sheet, kept while it is closed or a file is open.
    val filesStates = remember { HashMap<String, FilesState>() }
    val snackbar = remember { SnackbarHostState() }

    LaunchedEffect(Unit) {
        if (autoPair != null) pairing.pair(autoPair)
    }
    LaunchedEffect(record) {
        if (record == null) return@LaunchedEffect
        val connected = link.connect() ?: return@LaunchedEffect
        // A Host change can drop this link while the call is out; its error
        // must not escape the cancelled effect and end the app.
        terminals = runCatching { connected.terminals() }.getOrDefault(emptyList())
        onConnected()
    }
    // The terminals change as panes start and stop.
    LaunchedEffect(session, overview.terminals) {
        session?.let { current -> runCatching { terminals = current.terminals() } }
    }

    if (record == null) {
        PairScreen(pairing) { code -> scope.launch { pairing.pair(code) } }
        return
    }

    val nav = rememberNavController()
    val hostNames = link.hostNames(hosts)
    val hostLabel = hostNames.firstOrNull { it.first == hosts.current }?.second.orEmpty()

    /** Pushes where an Agent or its terminal runs. */
    suspend fun open(sessionId: String) {
        var target = destination(sessionId, overview, terminals)
        if (target == null) {
            session?.let { current -> runCatching { terminals = current.terminals() } }
            target = destination(sessionId, overview, terminals)
        }
        when (target) {
            is Destination.Project -> nav.navigate(Routes.project(target.place))
            is Destination.Terminal -> nav.navigate(Routes.terminal(target.info.sessionId))
            null -> snackbar.showSnackbar("这个终端已经不在了")
        }
    }

    // A tapped notification names a terminal: open it over 总控, so back returns there.
    LaunchedEffect(session, overview.projects.isNotEmpty()) {
        if (session == null) return@LaunchedEffect
        link.openRequest.collect { sessionId ->
            if (sessionId != null) {
                link.openRequest.value = null
                nav.showTopLevel(OVERVIEW)
                open(sessionId)
            }
        }
    }

    val entry by nav.currentBackStackEntryAsState()
    val route = entry?.destination?.route
    val attention = HomeSections(overview).attention.size
    Scaffold(
        contentWindowInsets = WindowInsets(0),
        snackbarHost = { SnackbarHost(snackbar) },
        bottomBar = {
            if (TOP_LEVEL.any { it.route == route }) {
                NavigationBar {
                    for (item in TOP_LEVEL) {
                        val selected = item.route == route
                        NavigationBarItem(
                            selected = selected,
                            onClick = { nav.showTopLevel(item.route) },
                            label = { Text(item.label) },
                            icon = {
                                BadgedBox(badge = {
                                    if (item.route == OVERVIEW && attention > 0) Badge { Text("$attention") }
                                }) {
                                    Icon(if (selected) item.selected else item.unselected, item.label)
                                }
                            },
                        )
                    }
                }
            }
        },
    ) { padding ->
        NavHost(nav, OVERVIEW, Modifier.padding(padding)) {
            composable(OVERVIEW) {
                OverviewScreen(
                    session = session,
                    hostLabel = hostLabel,
                    hosts = HostMenu(
                        names = hostNames,
                        current = hosts.current,
                        onSwitch = { id -> scope.launch { link.switchTo(id) } },
                        onAdd = { pairing.reset(); nav.navigate(PAIR) },
                        onManage = { nav.showTopLevel(SETTINGS) },
                    ),
                    status = linkStatus,
                    overview = overview,
                    onAgent = { agent -> scope.launch { open(agent.sessionId) } },
                    onRefresh = { session?.let { current -> runCatching { terminals = current.terminals() } } },
                )
            }
            composable(PROJECTS) {
                ProjectsScreen(overview) { id -> nav.navigate(Routes.project(overview.initialPlace(id))) }
            }
            composable(SETTINGS) {
                SettingsScreen(
                    hosts = hostNames,
                    current = hosts.current,
                    connected = session != null,
                    onSwitch = { id -> scope.launch { link.switchTo(id) } },
                    onForget = { id -> scope.launch { link.forget(id) } },
                    onAdd = { pairing.reset(); nav.navigate(PAIR) },
                    onBattery = onBattery,
                    onNotifications = onNotifications,
                    onLicenses = { nav.navigate(LICENSES) },
                )
            }
            composable(LICENSES) { LicensesScreen(onBack = { nav.popBackStack() }) }
            composable(PAIR) {
                PairScreen(pairing, onBack = { pairing.reset(); nav.popBackStack() }) { code ->
                    scope.launch { if (pairing.pair(code)) nav.popBackStack() }
                }
            }
            composable(
                Routes.PROJECT_PATTERN,
                arguments = listOf(optional(Routes.ARG_PROJECT), optional(Routes.ARG_TAB), optional(Routes.ARG_PANE)),
            ) { backStack ->
                val current = session
                if (current != null) {
                    ProjectScreen(
                        session = current,
                        link = link,
                        overview = overview,
                        terminals = terminals,
                        initial = Routes.place(backStack.arguments.toArguments(Routes.ARG_PROJECT, Routes.ARG_TAB, Routes.ARG_PANE)),
                        autoType = autoType,
                        filesStates = filesStates,
                        onBack = { nav.popBackStack() },
                        onHome = { nav.showTopLevel(OVERVIEW) },
                        onOpenFile = { project, path -> nav.navigate(Routes.editor(project, path)) },
                    )
                }
            }
            composable(Routes.TERMINAL_PATTERN, arguments = listOf(optional(Routes.ARG_SESSION))) { backStack ->
                val id = Routes.sessionId(backStack.arguments.toArguments(Routes.ARG_SESSION))
                val current = session
                val info = terminals.firstOrNull { it.sessionId == id }
                if (current != null && info != null) {
                    TerminalScreen(current, link, overview, info) { nav.popBackStack() }
                }
            }
            composable(Routes.EDITOR_PATTERN, arguments = listOf(optional(Routes.ARG_PROJECT), optional(Routes.ARG_PATH))) { backStack ->
                val (project, path) = Routes.file(backStack.arguments.toArguments(Routes.ARG_PROJECT, Routes.ARG_PATH))
                val current = session
                if (current != null) {
                    EditorScreen(current, project, overview.projectName(project), path) { nav.popBackStack() }
                }
            }
        }
    }
}
