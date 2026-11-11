import SwiftUI
import SwiftTerm
import UIKit

struct TerminalScreen: View {
    let session: HostSession
    let terminal: TerminalInfo
    @State private var controlling = false
    @State private var stats = ""
    @State private var error: String?

    var body: some View {
        ZStack(alignment: .bottom) {
            TerminalHost(session: session, terminal: terminal, controlling: controlling) { stats = $0 }
                .ignoresSafeArea(.container, edges: .bottom)
            if !controlling {
                HStack {
                    VStack(alignment: .leading) {
                        Text("正在观察这个终端").font(.headline)
                        Text("接管后可以输入，电脑上这个终端会改成手机尺寸")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("接管终端") {
                        Task {
                            do {
                                try await session.takeControl()
                                controlling = true
                            } catch {
                                self.error = error.localizedDescription
                            }
                        }
                    }
                    .buttonStyle(.borderedProminent)
                }
                .padding()
                .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 16))
                .padding()
            }
        }
        .overlay(alignment: .topTrailing) {
            Text(stats)
                .font(.system(size: 10, design: .monospaced))
                .padding(6)
                .background(.black.opacity(0.6), in: RoundedRectangle(cornerRadius: 6))
                .foregroundStyle(.white)
                .padding(8)
                .allowsHitTesting(false)
        }
        .navigationTitle(terminal.sessionId)
        .navigationBarTitleDisplayMode(.inline)
        .alert("接管失败", isPresented: .constant(error != nil)) {
            Button("好") { error = nil }
        } message: { Text(error ?? "") }
    }
}

/// Wraps SwiftTerm's view: drains repaints every display frame; while
/// observing it keeps the Host's column count inside a scroll view, while
/// controlling it fits the screen and sends keys and size changes.
struct TerminalHost: UIViewRepresentable {
    let session: HostSession
    let terminal: TerminalInfo
    let controlling: Bool
    let onStats: (String) -> Void

    func makeUIView(context: Context) -> TerminalContainer {
        TerminalContainer(session: session, terminal: terminal, onStats: onStats)
    }

    func updateUIView(_ view: TerminalContainer, context: Context) {
        view.setControlling(controlling)
    }

    static func dismantleUIView(_ view: TerminalContainer, coordinator: ()) {
        view.stop()
    }
}

final class TerminalContainer: UIView, TerminalViewDelegate {
    private let session: HostSession
    private let info: TerminalInfo
    private let onStats: (String) -> Void
    private let scroll = UIScrollView()
    private let terminalView = TerminalView(frame: .zero)
    private var link: CADisplayLink?
    private var controlling = false
    private var frames = 0
    private var lastReport = CACurrentMediaTime()
    private var fedBytes = 0

    init(session: HostSession, terminal: TerminalInfo, onStats: @escaping (String) -> Void) {
        self.session = session
        self.info = terminal
        self.onStats = onStats
        super.init(frame: .zero)
        backgroundColor = .black
        scroll.alwaysBounceHorizontal = true
        scroll.indicatorStyle = .white
        addSubview(scroll)
        terminalView.terminalDelegate = self
        terminalView.changeScrollback(0)
        terminalView.font = UIFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        scroll.addSubview(terminalView)
        observeLayout()
        Task { @MainActor in
            do {
                try await session.attach(sessionId: terminal.sessionId)
            } catch {
                onStats("attach failed: \(error.localizedDescription)")
            }
        }
        let link = CADisplayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    func stop() {
        link?.invalidate()
        link = nil
        let id = info.sessionId
        Task { try? await session.detach(sessionId: id) }
    }

    /// While observing, the component keeps the Host's size; the scroll view
    /// pans across it.
    private func observeLayout() {
        terminalView.resize(cols: Int(info.cols), rows: Int(info.rows))
        let size = terminalView.getOptimalFrameSize().size
        terminalView.frame = CGRect(origin: .zero, size: size)
        scroll.contentSize = size
    }

    func setControlling(_ controlling: Bool) {
        guard controlling != self.controlling else { return }
        self.controlling = controlling
        setNeedsLayout()
        if controlling {
            _ = terminalView.becomeFirstResponder()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        scroll.frame = bounds
        if controlling {
            terminalView.frame = scroll.bounds
            scroll.contentSize = scroll.bounds.size
        }
    }

    @objc private func tick() {
        frames += 1
        if let bytes = session.drain(sessionId: info.sessionId) {
            fedBytes += bytes.count
            terminalView.feed(byteArray: ArraySlice([UInt8](bytes)))
        }
        let now = CACurrentMediaTime()
        if now - lastReport >= 0.5 {
            let stats = session.drainStats()
            let fps = Double(frames) / (now - lastReport)
            let rate = Double(fedBytes) / (now - lastReport) / 1024
            onStats(String(format: "%.0f fps · %.0f KiB/s · drain p95 %llu µs max %llu µs",
                           fps, rate, stats.p95Micros, stats.maxMicros))
            frames = 0
            fedBytes = 0
            lastReport = now
        }
    }

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
