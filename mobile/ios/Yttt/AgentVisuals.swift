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

    var stateColor: Color {
        switch state {
        case .waiting: return .orange
        case .failed: return .red
        case .completed: return .green
        default: return .secondary
        }
    }
}
