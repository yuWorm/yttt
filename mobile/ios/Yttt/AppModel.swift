import Foundation
import Network
import UIKit
import Security

/// App-private storage: the paired Hosts in Application Support (see
/// `HostStore`), the device secret in the Keychain.
enum Storage {
    static var directory: URL {
        let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("yttt", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// Where the core keeps its connection log.
    static var logs: URL { directory.appendingPathComponent("logs", isDirectory: true) }

    /// The device's 32-byte secret, in the Keychain.
    static func deviceSecret() -> Data {
        let store = SecretStore(service: "com.yttt.yai", account: "device-secret")
        do { return try store.deviceSecret() } catch { fatalError("device secret: \(error)") }
    }
}

final class ShortCodeListener: PairingListener, @unchecked Sendable {
    private let onCode: @Sendable (UInt32) -> Void
    init(_ onCode: @escaping @Sendable (UInt32) -> Void) { self.onCode = onCode }
    func onShortCode(shortCode: UInt32) { onCode(shortCode) }
}

final class LinkSink: LinkListener, @unchecked Sendable {
    private let onLink: @Sendable (LinkState) -> Void
    init(_ onLink: @escaping @Sendable (LinkState) -> Void) { self.onLink = onLink }
    func onLink(state: LinkState) { onLink(state) }
}

final class OverviewSink: OverviewListener, @unchecked Sendable {
    private let onOverview: @Sendable (Overview) -> Void
    init(_ onOverview: @escaping @Sendable (Overview) -> Void) { self.onOverview = onOverview }
    func onOverview(overview: Overview) { onOverview(overview) }
}

/// The tab bar's three tabs.
enum AppTab: Hashable {
    case overview, projects, settings
}

@MainActor
final class AppModel: ObservableObject {
    let core: MobileCore
    let secret: Data
    /// The current Host's record.
    @Published var record: Data?
    /// The paired Hosts; the link is with the current one.
    @Published var hosts: HostList
    /// Pairing with one more Host while connected to another.
    @Published var addingHost = false
    private let hostStore: HostStore
    @Published var shortCode: UInt32?
    @Published var status = ""
    @Published var busy = false
    @Published var session: HostSession?
    @Published var terminals: [TerminalInfo] = []
    @Published var overview = Overview(projects: [], agents: [], terminals: 0, terminalIds: [])
    /// The selected tab, and the pages pushed in the two that push.
    @Published var tab: AppTab = .overview
    @Published var overviewPath: [Destination] = []
    @Published var projectsPath: [Destination] = []
    /// Direct or through the relay, refreshed while connected.
    @Published var linkPath: LinkPath = .unknown
    /// The signed-in relay account, if any: pairing and connecting go
    /// through the Host's relay node with its tickets.
    @Published var account: RelayAccount?
    /// Why the account was signed out, until the user signs in again.
    @Published var accountNotice: String?
    /// Where the current link's relay came from.
    @Published var relaySource: RelaySource = .none
    /// Whether the session reaches the Host, for the banner.
    @Published var link: LinkState = .ready
    /// When the link went down, for offering a fresh connection after a while.
    @Published var downSince: Date?
    /// The session this app made for each Host change, so views follow a new one.
    var sessionKey: String { session.map { String(describing: ObjectIdentifier($0)) } ?? "none" }
    private let accountStore = AccountStore()
    /// A project file open full screen.
    @Published var editing: EditingFile?
    /// Debug automation: the project page opens its files sheet once.
    var autoFilesPending = false

    private var automated = false
    /// Debug builds only: automation for simulator runs.
    static let hooks = LaunchHooks.current
    static var relayOnly: Bool { hooks.relayOnly }
    /// The code to pair with at launch, given or read from a QR code image
    /// through the same reader as the photo picker.
    static var autoPairCode: String? {
        hooks.pairCode ?? hooks.qrImage.flatMap { path in
            UIImage(contentsOfFile: path)?.cgImage.flatMap(pairingCode(in:))
        }
    }

    init() {
        startConnectionLog(dir: Storage.logs.path)
        let core = try! MobileCore()
        self.core = core
        secret = Storage.deviceSecret()
        hostStore = HostStore(directory: Storage.directory)
        let hosts = hostStore.load()
        self.hosts = hosts
        record = hosts.currentRecord
        account = accountStore.load()
    }

    /// Keeps a new sign-in and reconnects through its relay.
    func signIn(_ signedIn: SignedIn) async {
        let account = RelayAccount(signedIn)
        accountStore.save(account)
        self.account = account
        accountNotice = nil
        await reconnect()
    }

    /// Forgets the account; `notice` says why when the phone did it itself.
    func signOut(notice: String? = nil) async {
        accountStore.delete()
        account = nil
        accountNotice = notice
        await reconnect()
    }

    /// Tries the link again now; a session that gave up is made again.
    func retryNow() {
        if case .lost = link { Task { await reconnect() } } else { session?.retryNow() }
    }

    /// A fresh session, as leaving and coming back did.
    func reconnectFresh() async { await reconnect() }

    /// Back in the foreground: the network may have changed while suspended.
    func enterForeground() {
        backgroundRelease?.cancel()
        backgroundRelease = nil
        if releasedInBackground {
            releasedInBackground = false
            Task { await connect() }
            return
        }
        // The network may have changed meanwhile.
        session?.networkChanged()
    }

    private func follow(_ state: LinkState, of connected: HostSession) {
        guard session === connected else { return }
        let wasReady = link == .ready
        link = state
        switch state {
        case .ready:
            downSince = nil
            // Terminals may have changed while down (a Host restart).
            if !wasReady { Task { await refresh() } }
        case .lost:
            downSince = downSince ?? Date()
            // The session gave up; a new one may still get through, once.
            if wasReady { Task { await reconnect() } }
        case .connecting, .reconnecting:
            if downSince == nil {
                downSince = Date()
                scheduleFreshConnect(for: connected)
            }
        }
    }

    /// The last fresh connection this app made by itself, to space them out.
    private var lastAutoFresh: Date?

    /// Down this long in the foreground, a fresh session usually gets through
    /// where the old one keeps failing, as leaving and coming back did; at most
    /// once a minute.
    private func scheduleFreshConnect(for connected: HostSession) {
        Task {
            try? await Task.sleep(for: .seconds(20))
            guard session === connected, link != .ready,
                  UIApplication.shared.applicationState == .active,
                  pathMonitor.currentPath.status == .satisfied,
                  lastAutoFresh.map({ Date().timeIntervalSince($0) > 60 }) ?? true
            else { return }
            lastAutoFresh = Date()
            await reconnect()
        }
    }

    /// Connects to the current Host again, as after the account changed.
    private func reconnect() async {
        guard record != nil, !connecting else { return }
        let old = session
        session = nil
        await old?.disconnect()
        await connect()
    }

    /// Each Host's name, told apart when two share one.
    var hostNames: [(id: String, name: String)] {
        hosts.names { (try? core.hostInfo(record: $0).label) ?? "Host" }
    }

    var hostLabel: String { hostNames.first { $0.id == hosts.current }?.name ?? "" }

    /// Connects to another paired Host instead.
    func switchTo(_ id: String) async {
        guard id != hosts.current else { return }
        await changeHosts(hosts.selected(id))
    }

    /// The name to edit: the one given on this phone, else the Host's own.
    func editableName(_ id: String) -> String {
        guard let entry = hosts.hosts.first(where: { $0.id == id }) else { return "" }
        return entry.name ?? (try? core.hostInfo(record: entry.record).label) ?? ""
    }

    /// Names a Host on this phone only; blank goes back to the Host's own name.
    func rename(_ id: String, to name: String) {
        let list = hosts.renamed(id, name)
        hostStore.save(list)
        hosts = list
    }

    /// Forgets a Host. Forgetting the current one moves the link to the
    /// first one left, if any; another one just leaves the list.
    func forget(_ id: String) async {
        // Gives the key's device slot on the account back; the Host is
        // forgotten here either way.
        if let account, let record = hosts.hosts.first(where: { $0.id == id })?.record {
            try? await core.releaseHost(account: account.account, record: record, deviceSecret: secret)
        }
        let list = hosts.removed(id)
        if id == hosts.current {
            await changeHosts(list)
        } else {
            hostStore.save(list)
            hosts = list
        }
    }

    /// Drops the link and everything the old Host showed, stores the list,
    /// then connects to the new current Host.
    private func changeHosts(_ list: HostList) async {
        await session?.disconnect()
        session = nil
        terminals = []
        overview = Overview(projects: [], agents: [], terminals: 0, terminalIds: [])
        overviewPath = []
        projectsPath = []
        editing = nil
        hostStore.save(list)
        hosts = list
        record = list.currentRecord
        await connect()
    }

    func pair(code: String) async {
        busy = true
        shortCode = nil
        status = "正在连接 Host…"
        defer { busy = false }
        let listener = ShortCodeListener { code in
            Task { @MainActor in
                self.shortCode = code
                self.status = "在电脑上核对数字并确认"
            }
        }
        do {
            let record = try await core.pair(code: code, deviceLabel: UIDevice.current.name, deviceSecret: secret,
                                            relayOnly: Self.relayOnly, account: account?.account, listener: listener)
            let id = try core.hostInfo(record: record).id
            status = ""
            shortCode = nil
            addingHost = false
            await changeHosts(hosts.added(id, record))
        } catch MobileCoreError.SignInExpired {
            shortCode = nil
            status = "Relay 账号的登录已过期，请重新登录后再配对。"
            await signOut(notice: "Relay 账号的登录已过期，请重新登录。")
        } catch {
            shortCode = nil
            status = "配对失败：\(error.message)"
        }
    }

    private let pathMonitor = NWPathMonitor()
    private var pathDebounce: Task<Void, Never>?

    /// Tells the core when a usable network path appears or changes (Wi-Fi to
    /// cellular, back after an outage), debounced, so the session re-checks
    /// its link at once. Not when the path goes away: a probe then finds no
    /// answer and replaces a connection that would have outlived the outage.
    private func watchNetwork() {
        pathMonitor.pathUpdateHandler = { [weak self] path in
            guard path.status == .satisfied else { return }
            Task { @MainActor in
                guard let self else { return }
                self.pathDebounce?.cancel()
                self.pathDebounce = Task {
                    try? await Task.sleep(for: .milliseconds(500))
                    guard !Task.isCancelled else { return }
                    self.session?.networkChanged()
                }
            }
        }
        pathMonitor.start(queue: .main)
    }

    /// Whether a connect is under way: the home screen and a Host change
    /// both ask, and only one link should come of it.
    private var connecting = false

    func connect() async {
        guard session == nil, !connecting, let record else { return }
        connecting = true
        defer { connecting = false }
        if pathMonitor.pathUpdateHandler == nil { watchNetwork() }
        busy = true
        status = "正在连接…"
        defer { busy = false }
        let usedAccount = account
        do {
            let connected = try await core.connect(record: record, deviceSecret: secret, relayOnly: Self.relayOnly,
                                                   account: usedAccount?.account, deviceLabel: UIDevice.current.name)
            // The Host or the account changed while connecting (paired,
            // switched, forgot, signed in or out): this link is the old one;
            // connect again with what is current.
            guard self.record == record, self.account == usedAccount else {
                await connected.disconnect()
                connecting = false
                await connect()
                return
            }
            session = connected
            link = .ready
            downSince = nil
            connected.watchLink(listener: LinkSink { state in
                Task { @MainActor in self.follow(state, of: connected) }
            })
            relaySource = connected.relaySource()
            status = ""
            _ = try? await connected.openActiveWorkspace()
            connected.watchOverview(listener: OverviewSink { overview in
                Task { @MainActor in
                    // A link dropped since (a Host change) may still deliver one.
                    guard self.session === connected else { return }
                    self.apply(overview)
                }
            })
            await refresh()
            watchLinkPath()
        } catch MobileCoreError.SignInExpired {
            // Signing out connects again, without the account.
            connecting = false
            await signOut(notice: "Relay 账号的登录已过期，请重新登录。")
        } catch {
            guard self.record == record, self.account == usedAccount else {
                connecting = false
                await connect()
                return
            }
            status = "连接失败：\(error.message)"
        }
    }

    private var pathWatch: Task<Void, Never>?

    /// Follows whether the link goes direct or through the relay.
    private func watchLinkPath() {
        pathWatch?.cancel()
        pathWatch = Task {
            while !Task.isCancelled, let session {
                linkPath = await session.path()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    func refresh() async {
        guard let session else { return }
        guard link == .ready else { return retryNow() }
        let asked = session
        do {
            terminals = try await session.terminals()
            if overviewPath.isEmpty, let open = Self.hooks.open,
               let terminal = terminals.first(where: { $0.sessionId == open }) {
                overviewPath.append(.terminal(terminal))
            }
        } catch {
            // A link dropped since (a Host change, a reconnect) says nothing about this one.
            guard self.session === asked else { return }
            status = "读取终端失败：\(error.message)"
        }
    }

    /// Leaving the foreground gives every terminal back to the computer, in
    /// the time iOS grants a task started on the way out.
    /// How long the link stays up in the background, in the time iOS grants
    /// a task started on the way out: a quick trip to another app keeps it.
    static let keepLinkInBackground: Duration = .seconds(20)
    /// Lets go of the link once the app has been away a while.
    private var backgroundRelease: Task<Void, Never>?
    /// The link was let go in the background; a fresh one is made on return.
    private var releasedInBackground = false

    /// Leaving the foreground gives every terminal back to the computer. After
    /// a while away the link is closed cleanly: iOS would suspend the app and
    /// leave the connection to die unseen, which the phone then takes long to
    /// notice on return, while the Host waits out its own timeout. A fresh
    /// one on return is quick, as leaving and coming back always was.
    func enterBackground() {
        guard let session else { return }
        var task = UIBackgroundTaskIdentifier.invalid
        task = UIApplication.shared.beginBackgroundTask(withName: "keep-link") {
            UIApplication.shared.endBackgroundTask(task)
        }
        backgroundRelease?.cancel()
        backgroundRelease = Task {
            defer { UIApplication.shared.endBackgroundTask(task) }
            try? await session.giveBack(sessionIds: [])
            try? await Task.sleep(for: Self.keepLinkInBackground)
            guard !Task.isCancelled, self.session === session else { return }
            releasedInBackground = true
            self.session = nil
            link = .ready
            downSince = nil
            await session.disconnect()
        }
    }

    private func apply(_ overview: Overview) {
        // Ids, not the count: a restarted Host has new ones in the same number.
        let terminalsChanged = overview.terminalIds != self.overview.terminalIds
        self.overview = overview
        // The terminals change as panes start and stop.
        if terminalsChanged { Task { await refresh() } }
        if !automated, let project = Self.hooks.project,
           overview.projects.contains(where: { $0.projectId == project }) {
            automated = true
            var place = overview.initialPlace(project)
            if let tab = Self.hooks.tab { place = overview.placeIn(project, tab) }
            if let pane = Self.hooks.pane { place = Place(projectId: project, tabId: place.tabId, paneId: pane) }
            tab = .overview
            overviewPath = [.project(place)]
            autoFilesPending = Self.hooks.files
            if let path = Self.hooks.edit { editing = EditingFile(projectId: project, path: path) }
        }
    }

    /// Back to the overview, with nothing pushed.
    func goHome() {
        tab = .overview
        overviewPath = []
    }

    /// Opens where an Agent runs, over the overview: its pane, or its
    /// terminal alone.
    func open(agent: AgentSummary) async {
        var target = destination(agent.sessionId, overview, terminals)
        if target == nil {
            await refresh()
            target = destination(agent.sessionId, overview, terminals)
        }
        guard let target else {
            status = "这个 Agent 的终端已经不在了"
            return
        }
        tab = .overview
        overviewPath = [target]
    }
}
