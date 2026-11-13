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
    @Published var overview = Overview(projects: [], agents: [], terminals: 0)
    /// The selected tab, and the pages pushed in the two that push.
    @Published var tab: AppTab = .overview
    @Published var overviewPath: [Destination] = []
    @Published var projectsPath: [Destination] = []
    /// Direct or through the relay, refreshed while connected.
    @Published var linkPath: LinkPath = .unknown
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
        let core = try! MobileCore()
        self.core = core
        secret = Storage.deviceSecret()
        hostStore = HostStore(directory: Storage.directory)
        let hosts = hostStore.load()
        self.hosts = hosts
        record = hosts.currentRecord
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

    /// Forgets a Host. Forgetting the current one moves the link to the
    /// first one left, if any; another one just leaves the list.
    func forget(_ id: String) async {
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
        overview = Overview(projects: [], agents: [], terminals: 0)
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
                                            relayOnly: Self.relayOnly, listener: listener)
            let id = try core.hostInfo(record: record).id
            status = ""
            shortCode = nil
            addingHost = false
            await changeHosts(hosts.added(id, record))
        } catch {
            status = "配对失败：\(error.localizedDescription)"
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
        do {
            let connected = try await core.connect(record: record, deviceSecret: secret, relayOnly: Self.relayOnly)
            // The Host changed while connecting (paired, switched, forgot):
            // this link is to the old one; connect to the current one instead.
            guard self.record == record else {
                await connected.disconnect()
                connecting = false
                await connect()
                return
            }
            session = connected
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
        } catch {
            guard self.record == record else {
                connecting = false
                await connect()
                return
            }
            status = "连接失败：\(error.localizedDescription)"
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
            status = "读取终端失败：\(error.localizedDescription)"
        }
    }

    /// Leaving the foreground gives every terminal back to the computer, in
    /// the time iOS grants a task started on the way out.
    func enterBackground() {
        guard let session else { return }
        var task = UIBackgroundTaskIdentifier.invalid
        task = UIApplication.shared.beginBackgroundTask(withName: "give-back") {
            UIApplication.shared.endBackgroundTask(task)
        }
        Task {
            try? await session.giveBack(sessionIds: [])
            UIApplication.shared.endBackgroundTask(task)
        }
    }

    private func apply(_ overview: Overview) {
        let terminalsChanged = overview.terminals != self.overview.terminals
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
