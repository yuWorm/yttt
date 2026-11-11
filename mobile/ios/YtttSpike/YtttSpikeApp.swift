import SwiftUI

@main
struct YtttSpikeApp: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        WindowGroup {
            NavigationStack {
                if model.record == nil {
                    PairView()
                } else {
                    TerminalListView()
                }
            }
            .environmentObject(model)
        }
    }
}

struct PairView: View {
    @EnvironmentObject private var model: AppModel
    @State private var code = ""

    var body: some View {
        Form {
            Section("配对码") {
                TextField("YTTT-PAIR:1:…", text: $code, axis: .vertical)
                    .font(.system(.footnote, design: .monospaced))
                    .lineLimit(3...6)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("配对") { Task { await model.pair(code: code) } }
                    .disabled(model.busy || code.isEmpty)
            }
            if let shortCode = model.shortCode {
                Section("核对数字") {
                    Text(String(format: "%06d", shortCode))
                        .font(.system(.largeTitle, design: .monospaced))
                }
            }
            if !model.status.isEmpty {
                Section { Text(model.status).foregroundStyle(.secondary) }
            }
        }
        .navigationTitle("配对 Host")
    }
}

struct TerminalListView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        List {
            if !model.status.isEmpty {
                Text(model.status).foregroundStyle(.secondary)
            }
            if let session = model.session {
                ForEach(model.terminals, id: \.sessionId) { terminal in
                    NavigationLink {
                        TerminalScreen(session: session, terminal: terminal)
                    } label: {
                        VStack(alignment: .leading) {
                            Text(terminal.sessionId).font(.system(.body, design: .monospaced))
                            Text("\(terminal.cols)×\(terminal.rows) · \(terminal.projectId)")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
        .navigationTitle(model.hostLabel)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    Button("刷新") { Task { await model.refresh() } }
                    Button("忘记这台 Host", role: .destructive) { Task { await model.forget() } }
                } label: { Image(systemName: "ellipsis.circle") }
            }
        }
        .task { await model.connect() }
        .refreshable { await model.refresh() }
    }
}
