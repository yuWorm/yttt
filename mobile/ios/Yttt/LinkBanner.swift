import SwiftUI

/// A thin bar while the link is down: reconnecting, with a retry; after a
/// while, or once the session gave up, a fresh connection too.
struct LinkBanner: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        if model.session != nil, model.link != .ready {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let long = model.downSince.map { context.date.timeIntervalSince($0) > LinkBannerText.freshAfter } ?? false
                HStack(spacing: 10) {
                    if case .lost = model.link {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                    } else {
                        ProgressView().controlSize(.small)
                    }
                    Text(LinkBannerText.title(model.link)).font(.footnote).lineLimit(2)
                    Spacer(minLength: 4)
                    if long || isLost {
                        Button("重新连接") { Task { await model.reconnectFresh() } }
                            .font(.footnote.weight(.semibold))
                    } else {
                        Button("立即重试") { model.retryNow() }
                            .font(.footnote.weight(.semibold))
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .frame(maxWidth: .infinity)
                .background(.orange.opacity(0.15))
            }
            .transition(.move(edge: .top).combined(with: .opacity))
        }
    }

    private var isLost: Bool {
        if case .lost = model.link { return true }
        return false
    }
}
