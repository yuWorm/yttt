import Foundation
import Network
import UIKit
import Security

/// Spike storage: files in Application Support. The real app keeps the
/// device secret in the Keychain.
enum Storage {
    static var directory: URL {
        let url = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("yttt-spike", isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    static func load(_ name: String) -> Data? {
        try? Data(contentsOf: directory.appendingPathComponent(name))
    }

    /// Saved app-private and left out of backups: the record names the
    /// Host this phone pairs with, and is no use on another phone.
    static func save(_ name: String, _ data: Data?) {
        var url = directory.appendingPathComponent(name)
        guard let data else { try? FileManager.default.removeItem(at: url); return }
        try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
    }

    /// The device's 32-byte secret, in the Keychain.
    static func deviceSecret() -> Data {
        let store = SecretStore(service: "dev.yttt.spike", account: "device-secret",
                                legacyFile: directory.appendingPathComponent("device-secret"))
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

enum Root: Equatable {
    case home
    case project(Place)
}

@MainActor
final class AppModel: ObservableObject {
    let core: MobileCore
    let secret: Data
    @Published var record: Data?
    @Published var shortCode: UInt32?
    @Published var status = ""
    @Published var busy = false
    @Published var session: HostSession?
    @Published var terminals: [TerminalInfo] = []
    @Published var overview = Overview(projects: [], agents: [], terminals: 0)
    @Published var path: [TerminalInfo] = []
    /// The root page, switched from the left drawer.
    @Published var root: Root = .home
    @Published var drawerOpen = false
    /// The right drawer of the project page: files and sessions.
    @Published var drawerFilesOpen = false
    /// A project file open full screen.
    @Published var editing: EditingFile?

    /// Spike-only automation for simulator runs, set through
    /// `SIMCTL_CHILD_<name>`: pair with this code, open this terminal, then
    /// take control and type this line.
    static let autoPairCode = ProcessInfo.processInfo.environment["YTTT_SPIKE_PAIR_CODE"]
        ?? ProcessInfo.processInfo.environment["YTTT_SPIKE_QR_IMAGE"].flatMap { path in
            // Pair from a QR code image, through the same reader as the photo picker.
            UIImage(contentsOfFile: path)?.cgImage.flatMap(pairingCode(in:))
        }
    static let autoOpen = ProcessInfo.processInfo.environment["YTTT_SPIKE_OPEN"]
    static let autoType = ProcessInfo.processInfo.environment["YTTT_SPIKE_TYPE"]
    /// Open this project (and tab, pane) once the overview has it, and
    /// optionally show the drawer, for screenshots.
    static let autoProject = ProcessInfo.processInfo.environment["YTTT_SPIKE_PROJECT"]
    static let autoTab = ProcessInfo.processInfo.environment["YTTT_SPIKE_TAB"]
    static let autoPane = ProcessInfo.processInfo.environment["YTTT_SPIKE_PANE"]
    static let autoDrawer = ProcessInfo.processInfo.environment["YTTT_SPIKE_DRAWER"] == "1"
    /// Show the right drawer, or open this file, in the automated project.
    static let autoFiles = ProcessInfo.processInfo.environment["YTTT_SPIKE_FILES"] == "1"
    static let autoEdit = ProcessInfo.processInfo.environment["YTTT_SPIKE_EDIT"]
    private var automated = false
    /// Keep off direct paths: everything goes through the Host's relay.
    static let relayOnly = ProcessInfo.processInfo.environment["YTTT_SPIKE_RELAY_ONLY"] == "1"

    init() {
        core = try! MobileCore()
        secret = Storage.deviceSecret()
        record = Storage.load("host.json")
    }

    var hostLabel: String {
        guard let record else { return "" }
        return (try? core.hostInfo(record: record).label) ?? ""
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
            let record = try await core.pair(code: code, deviceLabel: "iOS spike", deviceSecret: secret,
                                            relayOnly: Self.relayOnly, listener: listener)
            Storage.save("host.json", record)
            self.record = record
            status = ""
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

    func connect() async {
        guard session == nil, let record else { return }
        if pathMonitor.pathUpdateHandler == nil { watchNetwork() }
        busy = true
        status = "正在连接…"
        defer { busy = false }
        do {
            session = try await core.connect(record: record, deviceSecret: secret, relayOnly: Self.relayOnly)
            status = ""
            _ = try? await session?.openActiveWorkspace()
            session?.watchOverview(listener: OverviewSink { overview in
                Task { @MainActor in self.apply(overview) }
            })
            await refresh()
        } catch {
            status = "连接失败：\(error.localizedDescription)"
        }
    }

    func refresh() async {
        guard let session else { return }
        do {
            terminals = try await session.terminals()
            if path.isEmpty, let open = Self.autoOpen,
               let terminal = terminals.first(where: { $0.sessionId == open }) {
                path.append(terminal)
            }
        } catch {
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
        if !automated, let project = Self.autoProject,
           overview.projects.contains(where: { $0.projectId == project }) {
            automated = true
            var place = overview.initialPlace(project)
            if let tab = Self.autoTab { place = overview.placeIn(project, tab) }
            if let pane = Self.autoPane { place = Place(projectId: project, tabId: place.tabId, paneId: pane) }
            root = .project(place)
            drawerOpen = Self.autoDrawer
            drawerFilesOpen = Self.autoFiles
            if let path = Self.autoEdit { editing = EditingFile(projectId: project, path: path) }
        }
    }

    var currentProject: String? {
        if case .project(let place) = root { return place.projectId }
        return nil
    }

    func goHome() {
        root = .home
        drawerOpen = false
    }

    func openProject(_ projectId: String) {
        root = .project(overview.initialPlace(projectId))
        drawerOpen = false
    }

    /// Opens the pane an Agent runs in, or its terminal alone.
    func open(agent: AgentSummary) async {
        if let place = overview.placeOf(agent.sessionId) {
            root = .project(place)
            return
        }
        if !terminals.contains(where: { $0.sessionId == agent.sessionId }) { await refresh() }
        if let terminal = terminals.first(where: { $0.sessionId == agent.sessionId }) {
            path.append(terminal)
        } else {
            status = "这个 Agent 的终端已经不在了"
        }
    }

    func forget() async {
        await session?.disconnect()
        session = nil
        terminals = []
        Storage.save("host.json", nil)
        record = nil
        root = .home
        drawerOpen = false
    }
}
