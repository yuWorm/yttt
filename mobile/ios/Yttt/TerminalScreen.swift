import SwiftUI
import SwiftTerm
import UIKit

/// Hands the screen the terminal view's fitted size when taking control.
final class TerminalHandle {
    weak var container: TerminalContainer?
}

/// Whether the phone controls one terminal: shared by the terminal, the bar
/// that takes it, and the navigation bar's "交还电脑".
@MainActor
final class TerminalControl: ObservableObject {
    let session: HostSession
    let sessionId: String
    let handle = TerminalHandle()
    @Published var controlling = false
    @Published var hostTooOld = false
    @Published var error: String?
    /// The system keyboard is up, for the button that puts it away.
    @Published var keyboardShown = false
    private var observers: [NSObjectProtocol] = []

    init(session: HostSession, sessionId: String) {
        self.session = session
        self.sessionId = sessionId
        let center = NotificationCenter.default
        observers = [
            center.addObserver(forName: UIResponder.keyboardWillShowNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.keyboardShown = true }
            },
            center.addObserver(forName: UIResponder.keyboardWillHideNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.keyboardShown = false }
            },
        ]
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    /// Puts the keyboard away; tapping the terminal brings it back.
    func hideKeyboard() {
        handle.container?.hideKeyboard()
    }

    /// Takes the terminal at the size the view has once it fits the screen;
    /// keys go out only after the Host applied that size.
    func take() async {
        guard let container = handle.container else { return }
        let (cols, rows) = container.fitScreen()
        do {
            try await session.takeControl(sessionId: sessionId, cols: cols, rows: rows)
            controlling = true
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
        } catch MobileCoreError.HostTooOld {
            container.observeHostSize()
            hostTooOld = true
        } catch {
            container.observeHostSize()
            self.error = error.message
        }
    }

    func giveBack() {
        controlling = false
        handle.container?.observeHostSize()
        let id = sessionId
        Task { try? await session.giveBack(sessionIds: [id]) }
    }
}

/// The terminal, observed at the Host's size until taken.
struct TerminalScreen: View {
    let terminal: TerminalInfo
    @ObservedObject var control: TerminalControl
    @State private var stats = ""
    @Environment(\.colorScheme) private var scheme
    @AppStorage(AppearanceSettings.terminalFontKey) private var font = AppearanceSettings.defaultFont
    @AppStorage(AppearanceSettings.terminalSizeKey) private var size = AppearanceSettings.defaultTerminalSize
    @AppStorage(AppearanceSettings.terminalThemeKey) private var themeId = AppearanceSettings.defaultTheme

    var body: some View {
        let look = TerminalLook(font: font, size: AppearanceSettings.terminalSize(size),
                                theme: theme(id: themeId, systemDark: scheme == .dark))
        TerminalHost(session: control.session, terminal: terminal, controlling: control.controlling,
                     look: look, handle: control.handle,
                     onStats: { stats = $0 },
                     onControlLost: { control.controlling = false })
            .overlay(alignment: .topTrailing) {
                #if DEBUG
                if !stats.isEmpty {
                    Text(stats)
                        .font(.system(size: 10, design: .monospaced))
                        .padding(6)
                        .background(.black.opacity(0.6), in: RoundedRectangle(cornerRadius: 6))
                        .foregroundStyle(.white)
                        .padding(8)
                        .allowsHitTesting(false)
                }
                #endif
            }
            .task {
                guard let line = AppModel.hooks.type else { return }
                try? await Task.sleep(for: .seconds(2))
                await control.take()
                guard control.controlling else { return }
                try? control.session.sendInput(sessionId: terminal.sessionId, bytes: Data((line + "\r").utf8))
            }
            .alert("接管失败", isPresented: .constant(control.error != nil)) {
                Button("好") { control.error = nil }
            } message: { Text(control.error ?? "") }
    }
}

/// The bar along the bottom while observing: what the Agent waits for, and
/// the button that takes the terminal.
struct TakeControlBar: View {
    let agent: AgentSummary?
    @ObservedObject var control: TerminalControl

    var body: some View {
        HStack(spacing: 10) {
            if control.hostTooOld {
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                Text("电脑上的 yttt 版本太旧，不能只接管一个终端。升级后再试。").font(.footnote)
                Spacer(minLength: 0)
            } else {
                if let agent, agent.state == .waiting {
                    IconTile(systemName: agent.stateSymbol, color: .orange)
                } else {
                    IconTile(systemName: "eye.fill", color: .gray)
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text(title).font(.subheadline.weight(.semibold))
                    Text("接管后可以输入，电脑上这个终端会变成手机的尺寸")
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 0)
                Button { Task { await control.take() } } label: {
                    Label("接管", systemImage: "keyboard")
                }
                .buttonStyle(.borderedProminent)
                .buttonBorderShape(.capsule)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
        .background(.bar)
    }

    private var title: String {
        guard let agent, agent.state == .waiting else { return "正在观察" }
        switch agent.waiting {
        case .approval: return "等批准"
        case .userInput: return "等回答"
        default: return "等待中"
        }
    }
}

/// Wraps SwiftTerm's view: drains repaints every display frame; while
/// observing it keeps the Host's column count inside a scroll view, while
/// controlling it fits the screen and sends keys and size changes.
struct TerminalHost: UIViewRepresentable {
    let session: HostSession
    let terminal: TerminalInfo
    let controlling: Bool
    let look: TerminalLook
    let handle: TerminalHandle
    let onStats: (String) -> Void
    let onControlLost: () -> Void

    func makeUIView(context: Context) -> TerminalContainer {
        let view = TerminalContainer(session: session, terminal: terminal, look: look, onStats: onStats,
                                     onControlLost: onControlLost)
        handle.container = view
        return view
    }

    func updateUIView(_ view: TerminalContainer, context: Context) {
        view.apply(look)
        view.setControlling(controlling)
    }

    static func dismantleUIView(_ view: TerminalContainer, coordinator: ()) {
        view.stop()
    }
}

final class TerminalContainer: UIView, TerminalViewDelegate, UIGestureRecognizerDelegate {
    private let session: HostSession
    private let info: TerminalInfo
    private let onStats: (String) -> Void
    private let onControlLost: () -> Void
    private let scroll = UIScrollView()
    private let terminalView = TerminalView(frame: .zero)
    private var link: CADisplayLink?
    private var controlling = false
    /// Sized to the screen while a take is in flight.
    private var fitting = false
    /// Direct or relayed, refreshed with the stats.
    private var pathLabel = "—"
    /// One character cell of the current font.
    private var cell = CGSize(width: 7, height: 14)
    private var frames = 0
    private var lastReport = CACurrentMediaTime()
    private var fedBytes = 0
    private var look: TerminalLook?

    init(session: HostSession, terminal: TerminalInfo, look: TerminalLook, onStats: @escaping (String) -> Void,
         onControlLost: @escaping () -> Void) {
        self.session = session
        self.info = terminal
        self.onStats = onStats
        self.onControlLost = onControlLost
        super.init(frame: .zero)
        scroll.alwaysBounceHorizontal = true
        addSubview(scroll)
        terminalView.terminalDelegate = self
        terminalView.changeScrollback(0)
        scroll.addSubview(terminalView)
        apply(look)
        // A swipe down puts the keyboard away, as in Messages.
        let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swipedDown))
        swipe.direction = .down
        swipe.delegate = self
        addGestureRecognizer(swipe)
        Task { @MainActor in
            do {
                try await session.attach(sessionId: terminal.sessionId)
            } catch {
                onStats("attach failed: \(error.message)")
            }
        }
        let link = CADisplayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    /// The font and colors; a new font changes the grid, as the Host or the
    /// screen decides.
    func apply(_ look: TerminalLook) {
        guard look != self.look else { return }
        let fontChanged = look.font != self.look?.font || look.size != self.look?.size
        self.look = look
        let theme = look.theme
        if fontChanged { terminalView.font = look.uiFont }
        terminalView.installColors(theme.ansi.map { rgb in
            SwiftTerm.Color(red: UInt16((rgb >> 16) & 0xFF) * 257, green: UInt16((rgb >> 8) & 0xFF) * 257,
                            blue: UInt16(rgb & 0xFF) * 257)
        })
        terminalView.nativeForegroundColor = UIColor(rgb: theme.foreground)
        terminalView.nativeBackgroundColor = UIColor(rgb: theme.background)
        terminalView.caretColor = UIColor(rgb: theme.cursor)
        terminalView.selectedTextBackgroundColor = UIColor(rgb: theme.selection)
        backgroundColor = UIColor(rgb: theme.background)
        scroll.indicatorStyle = theme.dark ? .white : .black
        if fontChanged {
            if controlling || fitting {
                setNeedsLayout()
            } else {
                observeLayout()
            }
        }
        // Repaint everything in the new colors.
        session.repaint(sessionId: info.sessionId)
    }

    func hideKeyboard() {
        _ = terminalView.resignFirstResponder()
    }

    @objc private func swipedDown() {
        if controlling { hideKeyboard() }
    }

    func stop() {
        link?.invalidate()
        link = nil
        let id = info.sessionId
        Task { try? await session.detach(sessionId: id) }
    }

    /// Lays the terminal out to fit the screen ahead of taking control and
    /// returns the grid that fits.
    func fitScreen() -> (UInt16, UInt16) {
        fitting = true
        terminalView.frame = scroll.bounds
        scroll.contentSize = scroll.bounds.size
        // The grid SwiftTerm itself derives from this frame.
        let cols = Int(scroll.bounds.width / cell.width)
        let rows = Int(scroll.bounds.height / cell.height)
        return (UInt16(max(cols, 1)), UInt16(max(rows, 1)))
    }

    /// Back to watching at the Host's size.
    func observeHostSize() {
        fitting = false
        controlling = false
        observeLayout()
    }

    /// While observing, the component keeps the Host's size; the scroll view
    /// pans across it.
    private func observeLayout() {
        terminalView.resize(cols: Int(info.cols), rows: Int(info.rows))
        let size = terminalView.getOptimalFrameSize().size
        cell = CGSize(width: size.width / CGFloat(max(info.cols, 1)),
                      height: size.height / CGFloat(max(info.rows, 1)))
        terminalView.frame = CGRect(origin: .zero, size: size)
        scroll.contentSize = size
    }

    func setControlling(_ controlling: Bool) {
        guard controlling != self.controlling else { return }
        self.controlling = controlling
        if controlling {
            setNeedsLayout()
            _ = terminalView.becomeFirstResponder()
        } else {
            _ = terminalView.resignFirstResponder()
            observeHostSize()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        scroll.frame = bounds
        if controlling || fitting {
            terminalView.frame = scroll.bounds
            scroll.contentSize = scroll.bounds.size
        }
    }

    @objc private func tick() {
        frames += 1
        // The computer took the terminal back, or the connection dropped.
        if controlling && !session.inControl(sessionId: info.sessionId) {
            onControlLost()
        }
        if let bytes = session.drain(sessionId: info.sessionId) {
            fedBytes += bytes.count
            terminalView.feed(byteArray: ArraySlice([UInt8](bytes)))
        }
        let now = CACurrentMediaTime()
        if now - lastReport >= 0.5 {
            let stats = session.drainStats()
            let fps = Double(frames) / (now - lastReport)
            let rate = Double(fedBytes) / (now - lastReport) / 1024
            onStats(String(format: "%@ · %.0f fps · %.0f KiB/s · drain p95 %llu µs",
                           pathLabel, fps, rate, stats.p95Micros))
            Task { @MainActor in
                switch await session.path() {
                case .direct: pathLabel = "直连"
                case .relay: pathLabel = "经 relay"
                case .unknown: pathLabel = "—"
                }
            }
            frames = 0
            fedBytes = 0
            lastReport = now
        }
    }

    /// The swipe goes along with the terminal's own touches.
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

    // MARK: TerminalViewDelegate

    func send(source: TerminalView, data: ArraySlice<UInt8>) {
        guard controlling else { return }
        try? session.sendInput(sessionId: info.sessionId, bytes: Data(data))
    }

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        guard controlling, newCols > 0, newRows > 0 else { return }
        let id = info.sessionId
        Task { try? await session.resize(sessionId: id, cols: UInt16(newCols), rows: UInt16(newRows)) }
    }

    func setTerminalTitle(source: TerminalView, title: String) {}
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    func scrolled(source: TerminalView, position: Double) {}
    func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        if let url = URL(string: link) { UIApplication.shared.open(url) }
    }
    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
}
