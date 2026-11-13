import Foundation

/// The banner's words for a link state, and when it offers a fresh connection.
enum LinkBannerText {
    /// Down this long, the banner offers a fresh connection instead of a retry.
    static let freshAfter: TimeInterval = 15

    static func title(_ state: LinkState) -> String {
        switch state {
        case .ready: ""
        case .connecting: "正在连接…"
        case .reconnecting(let attempt, _): attempt > 1 ? "连接断开，正在重连（第 \(attempt) 次）…" : "连接断开，正在重连…"
        case .lost(let reason): "连接已断开：\(reason)"
        }
    }
}
