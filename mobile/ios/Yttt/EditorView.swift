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
    @Environment(\.colorScheme) private var scheme
    @AppStorage(AppearanceSettings.editorFontKey) private var font = AppearanceSettings.defaultFont
    @AppStorage(AppearanceSettings.editorSizeKey) private var size = AppearanceSettings.defaultEditorSize
    @AppStorage(AppearanceSettings.editorThemeKey) private var themeId = AppearanceSettings.defaultTheme
    @AppStorage(AppearanceSettings.editorWrapKey) private var wrap = false
    @AppStorage(AppearanceSettings.editorLineNumbersKey) private var lineNumbers = true

    private var look: EditorLook {
        EditorLook(font: font, size: AppearanceSettings.editorSize(size),
                   theme: theme(id: themeId, systemDark: scheme == .dark), wrap: wrap, lineNumbers: lineNumbers)
    }

    var body: some View {
        NavigationStack {
            Group {
                if text != nil {
                    RunestoneEditor(handle: editor, look: look, editable: true, onChange: { dirty = true })
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
            editor.setText(opened.text, language: editorLanguage(path: file.path))
            dirty = false
            status = ""
        } catch {
            status = "打不开：\(error.message)"
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
            status = "保存失败：\(error.message)"
        }
    }
}

/// Reaches the Runestone view from SwiftUI: its text, setting it with the
/// language to highlight it as, and the look it is drawn with.
final class EditorHandle {
    weak var view: TextView?
    private var pending: (text: String, language: String?)?
    private var language: TreeSitterLanguage?
    private(set) var theme: EditorTheme?

    var text: String? { view?.text }

    func setText(_ text: String, language id: String?) {
        language = treeSitterLanguage(id)
        if let view { load(text, into: view) } else { pending = (text, id) }
    }

    func attach(_ view: TextView, look: EditorLook) {
        self.view = view
        apply(look, to: view)
        if let pending {
            language = treeSitterLanguage(pending.language)
            load(pending.text, into: view)
        }
        pending = nil
    }

    /// A new theme needs the text parsed again with it; other settings apply at once.
    func apply(_ look: EditorLook, to view: TextView) {
        view.showLineNumbers = look.lineNumbers
        view.isLineWrappingEnabled = look.wrap
        guard theme?.look != look else { return }
        let theme = EditorTheme(look)
        self.theme = theme
        view.backgroundColor = theme.backgroundColor
        view.insertionPointColor = UIColor(rgb: look.theme.cursor)
        view.selectionBarColor = UIColor(rgb: look.theme.cursor)
        view.selectionHighlightColor = UIColor(rgb: look.theme.selection).withAlphaComponent(0.6)
        let text = view.text
        load(text, into: view)
    }

    private func load(_ text: String, into view: TextView) {
        guard let theme else { return }
        let selection = view.selectedRange
        if let language {
            view.setState(TextViewState(text: text, theme: theme, language: language))
        } else {
            view.setState(TextViewState(text: text, theme: theme))
        }
        if selection.upperBound <= (text as NSString).length { view.selectedRange = selection }
    }
}

struct RunestoneEditor: UIViewRepresentable {
    let handle: EditorHandle
    let look: EditorLook
    let editable: Bool
    let onChange: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator(onChange: onChange) }

    func makeUIView(context: Context) -> TextView {
        let view = TextView()
        view.lineSelectionDisplayType = .line
        view.isEditable = editable
        view.autocorrectionType = .no
        view.autocapitalizationType = .none
        view.smartQuotesType = .no
        view.smartDashesType = .no
        view.editorDelegate = context.coordinator
        handle.attach(view, look: look)
        return view
    }

    func updateUIView(_ view: TextView, context: Context) {
        handle.apply(look, to: view)
    }

    final class Coordinator: TextViewDelegate {
        let onChange: () -> Void
        init(onChange: @escaping () -> Void) { self.onChange = onChange }
        func textViewDidChange(_ textView: TextView) { onChange() }
    }
}
