import Runestone
import SwiftUI
import UIKit

struct EditingFile: Identifiable, Equatable {
    let projectId: String
    let path: String
    var id: String { projectId + ":" + path }
}

/// A project file, full screen, in Runestone. Saving checks the file is as
/// it was when opened; if the Host's copy changed, the user overwrites it or
/// reloads it. Closing with unsaved changes asks first.
struct EditorView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismiss) private var dismiss
    let file: EditingFile
    @State private var text: String?
    @State private var editor = EditorHandle()
    @State private var dirty = false
    @State private var status = ""
    @State private var conflict = false
    @State private var confirmClose = false

    var body: some View {
        NavigationStack {
            Group {
                if text != nil {
                    RunestoneEditor(handle: editor, onChange: { dirty = true })
                } else {
                    ProgressView()
                }
            }
            .navigationTitle((file.path.split(separator: "/").last.map(String.init) ?? file.path) + (dirty ? " ●" : ""))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("关闭") { if dirty { confirmClose = true } else { dismiss() } }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("保存") { Task { await save(force: false) } }.disabled(!dirty)
                }
                if !status.isEmpty {
                    ToolbarItem(placement: .bottomBar) { Text(status).font(.caption) }
                }
            }
        }
        .task { await load() }
        .alert("文件在电脑上改过了", isPresented: $conflict) {
            Button("覆盖", role: .destructive) { Task { await save(force: true) } }
            Button("重新加载") { Task { await load() } }
            Button("取消", role: .cancel) {}
        } message: {
            Text("覆盖会用手机上的内容替换电脑上的版本；重新加载会丢掉手机上的修改。")
        }
        .alert("放弃修改？", isPresented: $confirmClose) {
            Button("放弃", role: .destructive) { dismiss() }
            Button("继续编辑", role: .cancel) {}
        } message: { Text("这个文件还有没保存的修改。") }
    }

    private func load() async {
        guard let session = model.session else { return }
        do {
            let opened = try await session.openFile(projectId: file.projectId, path: file.path)
            text = opened.text
            editor.setText(opened.text)
            dirty = false
            status = ""
        } catch {
            status = "打不开：\(error.localizedDescription)"
        }
    }

    private func save(force: Bool) async {
        guard let session = model.session, let current = editor.text else { return }
        do {
            switch try await session.saveFile(projectId: file.projectId, path: file.path, text: current, force: force) {
            case .saved:
                dirty = false
                status = "已保存"
            case .conflict:
                conflict = true
            }
        } catch {
            status = "保存失败：\(error.localizedDescription)"
        }
    }
}

/// Reaches the Runestone view from SwiftUI: its text, and setting it.
final class EditorHandle {
    weak var view: TextView?
    private var pending: String?

    var text: String? { view?.text }

    func setText(_ text: String) {
        if let view { view.text = text } else { pending = text }
    }

    func attach(_ view: TextView) {
        self.view = view
        if let pending { view.text = pending }
        pending = nil
    }
}

struct RunestoneEditor: UIViewRepresentable {
    let handle: EditorHandle
    let onChange: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onChange: onChange) }

    func makeUIView(context: Context) -> TextView {
        let view = TextView()
        view.backgroundColor = .systemBackground
        view.showLineNumbers = true
        view.lineSelectionDisplayType = .line
        view.isLineWrappingEnabled = false
        view.autocorrectionType = .no
        view.autocapitalizationType = .none
        view.smartQuotesType = .no
        view.smartDashesType = .no
        view.editorDelegate = context.coordinator
        handle.attach(view)
        return view
    }

    func updateUIView(_ view: TextView, context: Context) {}

    final class Coordinator: TextViewDelegate {
        let onChange: () -> Void
        init(onChange: @escaping () -> Void) { self.onChange = onChange }
        func textViewDidChange(_ textView: TextView) { onChange() }
    }
}
