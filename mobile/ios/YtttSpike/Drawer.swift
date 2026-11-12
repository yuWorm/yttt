import SwiftUI

/// A drawer that slides in over `main` from the leading (or trailing) edge,
/// as on Android: iOS has none built in. Opens with a drag that starts at
/// that edge (or by setting `isOpen`); closes on tapping the scrim or dragging
/// it back.
struct SideDrawer<Main: View, Drawer: View>: View {
    var edge: HorizontalEdge = .leading
    @Binding var isOpen: Bool
    @ViewBuilder let main: Main
    @ViewBuilder let drawer: Drawer
    @GestureState private var drag: CGFloat = 0

    var body: some View {
        GeometryReader { geometry in
            let width = min(geometry.size.width * 0.82, 340)
            // Positive towards the drawer's open side.
            let sign: CGFloat = edge == .leading ? 1 : -1
            let pull = drag * sign
            let hidden = isOpen ? max(0, -pull) : width - max(0, min(pull, width))
            let shown = (width - hidden) / width
            ZStack(alignment: edge == .leading ? .leading : .trailing) {
                main
                    .allowsHitTesting(!isOpen)
                Color.black.opacity(0.35 * shown)
                    .ignoresSafeArea()
                    .allowsHitTesting(isOpen)
                    .onTapGesture { isOpen = false }
                    .gesture(closing(width, sign))
                drawer
                    .frame(width: width)
                    .frame(maxHeight: .infinity)
                    .background(Color(uiColor: .systemGroupedBackground))
                    .offset(x: -hidden * sign)
                    .gesture(closing(width, sign))
                // The edge a drag opens it from, about 20 pt wide.
                if !isOpen {
                    Color.clear
                        .frame(width: 20)
                        .frame(maxHeight: .infinity)
                        .contentShape(Rectangle())
                        .gesture(
                            DragGesture(minimumDistance: 8)
                                .updating($drag) { value, state, _ in state = value.translation.width }
                                .onEnded { value in if value.translation.width * sign > width / 3 { isOpen = true } }
                        )
                }
            }
            .animation(.snappy(duration: 0.25), value: isOpen)
        }
    }

    private func closing(_ width: CGFloat, _ sign: CGFloat) -> some Gesture {
        DragGesture(minimumDistance: 8)
            .updating($drag) { value, state, _ in state = value.translation.width }
            .onEnded { value in if value.translation.width * sign < -width / 3 { isOpen = false } }
    }
}

/// The left drawer: the overview, the projects with their counts, the Host.
struct LeftDrawerView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let badges = model.overview.badges()
        List {
            Section {
                Text(model.hostLabel).font(.footnote).foregroundStyle(.secondary).lineLimit(1)
                Button { model.goHome() } label: {
                    Label("总控", systemImage: "square.grid.2x2")
                }
                .fontWeight(model.root == .home ? .semibold : .regular)
            }
            Section("项目") {
                ForEach(model.overview.projects, id: \.projectId) { project in
                    Button { model.openProject(project.projectId) } label: {
                        HStack {
                            Text(project.name)
                                .fontWeight(model.currentProject == project.projectId ? .semibold : .regular)
                            Spacer()
                            if let badge = badges[project.projectId] {
                                if badge.waiting > 0 { count("等 \(badge.waiting)", .red) }
                                if badge.failed > 0 { count("败 \(badge.failed)", .orange) }
                                if badge.running > 0 { count("运行 \(badge.running)", .secondary) }
                            }
                        }
                    }
                }
            }
            Section {
                Button("忘记这台 Host", role: .destructive) { Task { await model.forget() } }
            }
        }
        .foregroundStyle(.primary)
    }

    private func count(_ text: String, _ color: Color) -> some View {
        Text(text)
            .font(.caption2.bold())
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .foregroundStyle(color == .secondary ? Color.primary : .white)
            .background(color == .secondary ? Color(uiColor: .systemFill) : color, in: Capsule())
    }
}
