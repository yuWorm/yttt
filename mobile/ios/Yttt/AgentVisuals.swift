import SwiftUI

/// The providers with a logo in Assets.xcassets/Agents (scripts/build-mobile-agent-logos.py).
private let logos: Set<String> = ["codex", "claude", "grok", "opencode", "pi", "omp"]

/// The name the desktop gives a provider.
func providerName(_ provider: String) -> String {
    switch provider {
    case "codex": return "Codex"
    case "claude": return "Claude Code"
    case "grok": return "Grok"
    case "opencode": return "OpenCode"
    case "pi": return "Pi"
    case "omp": return "Oh My Pi"
    default: return provider
    }
}

/// The provider's logo: Codex and Claude in their colours, the others as
/// templates in the foreground colour, as the desktop draws them; a CPU
/// symbol for any other provider.
struct AgentLogo: View {
    let provider: String?
    var size: CGFloat = 20

    var body: some View {
        Group {
            if let provider, logos.contains(provider) {
                Image("agent-\(provider)").resizable().scaledToFit()
            } else {
                Image(systemName: "cpu").resizable().scaledToFit()
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel(provider.map(providerName) ?? "Agent")
    }
}

/// The logo on a rounded tile, as a row's leading icon.
struct AgentTile: View {
    let provider: String?
    var size: CGFloat = 20

    var body: some View {
        AgentLogo(provider: provider, size: size)
            .padding(6)
            .background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

/// A white symbol on a coloured rounded square, as in the Settings app.
struct IconTile: View {
    let systemName: String
    let color: Color

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(.white)
            .frame(width: 29, height: 29)
            .background(color, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
    }
}

extension AgentSummary {
    var isUrgent: Bool { state == .waiting || state == .failed }

    /// A raised hand for approval, a bubble for a question, a bolt while working.
    var stateSymbol: String {
        switch state {
        case .waiting: return waiting == .userInput ? "bubble.left.fill" : "hand.raised.fill"
        case .failed: return "exclamationmark.octagon.fill"
        case .working: return "bolt.fill"
        case .starting: return "hourglass"
        case .idle: return "pause.circle.fill"
        case .completed, .interrupted, .stale: return "checkmark.circle.fill"
        }
    }

    /// Working or starting: shown in blue, with a light sweeping across.
    var isRunning: Bool { state == .working || state == .starting }

    var stateColor: Color {
        switch state {
        case .waiting: return .orange
        case .failed: return .red
        case .completed: return .green
        case .working, .starting: return .blue
        default: return .secondary
        }
    }
}

/// A light sweeping across the content, over and over, while `active`;
/// still when Reduce Motion is on.
struct Shimmer: ViewModifier {
    var active: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var phase: CGFloat = 0

    func body(content: Content) -> some View {
        if active && !reduceMotion {
            content
                .overlay {
                    GeometryReader { geometry in
                        let width = geometry.size.width
                        LinearGradient(colors: [.clear, .white.opacity(0.9), .clear],
                                       startPoint: .leading, endPoint: .trailing)
                            .frame(width: width * 0.6)
                            .offset(x: -width * 0.6 + phase * width * 1.6)
                    }
                    .mask(content)
                    .allowsHitTesting(false)
                }
                .onAppear {
                    phase = 0
                    withAnimation(.linear(duration: 1.4).repeatForever(autoreverses: false)) { phase = 1 }
                }
        } else {
            content
        }
    }
}

extension View {
    func shimmer(_ active: Bool = true) -> some View { modifier(Shimmer(active: active)) }
}

/// An Agent's state as a coloured label; a running one shimmers.
struct AgentStateLabel: View {
    let agent: AgentSummary

    var body: some View {
        Label(agent.stateLabel, systemImage: agent.stateSymbol)
            .font(.caption.weight(.semibold))
            .foregroundStyle(agent.stateColor)
            .labelStyle(.titleAndIcon)
            .symbolEffect(.pulse, isActive: agent.isRunning)
            .shimmer(agent.isRunning)
    }
}
