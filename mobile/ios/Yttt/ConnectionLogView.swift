import SwiftUI

/// The core's connection log, newest at the bottom, to read or share when
/// connecting was slow or failed.
struct ConnectionLogView: View {
    @State private var text = ""

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                Text(text.isEmpty ? "还没有记录。" : text)
                    .font(.system(size: 10, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
                Color.clear.frame(height: 1).id("end")
            }
            .onAppear { load(reader) }
            .refreshable { load(reader) }
        }
        .navigationTitle("连接日志")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                ShareLink(item: text, subject: Text("yttt 连接日志")) { Image(systemName: "square.and.arrow.up") }
                    .disabled(text.isEmpty)
            }
        }
    }

    private func load(_ reader: ScrollViewProxy) {
        text = readConnectionLog(dir: Storage.logs.path)
        DispatchQueue.main.async { reader.scrollTo("end", anchor: .bottom) }
    }
}
