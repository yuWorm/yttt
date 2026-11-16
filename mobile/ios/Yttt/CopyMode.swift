import SwiftUI
import UIKit

/// The terminal's text to select and copy with the system's handles, opened
/// by a long press with the pressed line selected. Scrolling up reads
/// further back: the Host's history, or, for a program that scrolls itself,
/// the pages it shows as the phone scrolls it, stitched together.
struct CopyModeView: View {
    @ObservedObject var control: TerminalControl
    let look: TerminalLook
    /// The screen row the long press was on.
    let pressedRow: Int?
    @Environment(\.dismiss) private var dismiss
    @State private var lines: [String] = []
    @State private var older: TextCursor?
    @State private var alternate = false
    @State private var loading = false
    @State private var atStart = false
    @State private var note: String?
    @State private var copied = false
    @State private var textHandle = CopyTextHandle()
    /// The program's screen last read, to stitch the next page above.
    @State private var screen: [String] = []
    @State private var firstPage = true
    /// Steps the program was scrolled up, undone on leaving.
    @State private var stepsUp: Int32 = 0

    /// Screens of history read at a time.
    private static let pages: UInt32 = 5

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let banner {
                    HStack {
                        Text(banner).font(.footnote).foregroundStyle(.secondary)
                        Spacer(minLength: 0)
                        if alternate && !control.controlling && !atStart {
                            Button("接管") { Task { await takeAndReload() } }
                                .font(.footnote.bold())
                        }
                    }
                    .padding(.horizontal).padding(.vertical, 8)
                    .background(.bar)
                }
                CopyTextView(text: lines.joined(separator: "\n"), look: look, handle: textHandle,
                             onNearTop: loadOlder)
            }
            .navigationTitle("复制")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("完成") { dismiss() } }
                ToolbarItemGroup(placement: .bottomBar) {
                    Button("全选") { textHandle.selectAll() }
                    Spacer()
                    Button(copied ? "已复制" : "复制") { copy() }.bold()
                }
            }
        }
        .task { await load() }
        .onDisappear(perform: scrollBack)
    }

    private var banner: String? {
        if let note { return note }
        if atStart { return alternate ? "已到程序内容的开头" : "已到历史开头" }
        if alternate {
            return control.controlling ? "往上滑会让程序往回翻页，拼接更早的内容"
                : "接管后往上滑，可以加载程序更早的内容"
        }
        return nil
    }

    /// Takes the terminal; it then shows at the phone's size, so the text
    /// starts over from the redrawn screen.
    private func takeAndReload() async {
        await control.take()
        guard control.controlling else { return }
        try? await Task.sleep(for: .milliseconds(600))
        firstPage = true
        stepsUp = 0
        await load()
    }

    private func load() async {
        do {
            let text = try await control.session.terminalText(sessionId: control.sessionId,
                                                              pages: Self.pages)
            older = text.older
            alternate = text.alternateScreen
            atStart = !alternate && text.older == nil
            screen = text.lines
            if let pressedRow, text.rowLines.indices.contains(pressedRow) {
                textHandle.selectLine(Int(text.rowLines[pressedRow]), of: text.lines)
            } else {
                textHandle.scrollToBottom()
            }
            lines = text.lines
        } catch {
            note = "读取失败：\(error.message)"
        }
    }

    private func loadOlder() {
        guard !loading, !atStart else { return }
        if alternate {
            pageUp()
        } else if let older {
            loadHistory(above: older)
        }
    }

    private func loadHistory(above cursor: TextCursor) {
        loading = true
        Task {
            defer { loading = false }
            do {
                let text = try await control.session.olderText(sessionId: control.sessionId,
                                                               cursor: cursor, pages: Self.pages)
                var merged = text.lines
                var rest = lines[...]
                if text.lastLineContinues, !merged.isEmpty, let first = rest.first {
                    merged[merged.count - 1] += first
                    rest = rest.dropFirst()
                }
                textHandle.keepingPlace(old: lines) { lines = merged + rest }
                older = text.older
                atStart = text.older == nil
            } catch {
                older = nil
                note = "历史已变化，关闭后重新打开可以重新读取"
            }
        }
    }

    /// Scrolls the program up a step and stitches what it shows above.
    private func pageUp() {
        guard control.controlling else { return }
        loading = true
        Task {
            defer { loading = false }
            do {
                let paged = try await control.session.pageUpText(sessionId: control.sessionId,
                                                                 previous: screen)
                guard !paged.atTop else {
                    atStart = true
                    return
                }
                stepsUp += 1
                // The program's title still starts the text until the first
                // page is stitched above it.
                let drop = firstPage ? Int(paged.header) : 0
                firstPage = false
                textHandle.keepingPlace(old: lines) {
                    lines = paged.above + lines.dropFirst(min(drop, lines.count))
                }
                screen = paged.screen
            } catch {
                note = error.message
            }
        }
    }

    /// Puts the program back where it was, once copying ends.
    private func scrollBack() {
        guard stepsUp > 0, control.controlling else { return }
        // One more: the last step up may have moved less than a full step,
        // and the program stops at its end.
        try? control.session.scrollProgram(sessionId: control.sessionId, pages: stepsUp + 1)
    }

    private func copy() {
        let text = textHandle.selectedText ?? lines.joined(separator: "\n")
        UIPasteboard.general.string = text
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        copied = true
        Task {
            try? await Task.sleep(for: .seconds(1.2))
            copied = false
        }
    }
}

/// Reaches the text view from the bar's buttons, and keeps the place and
/// the selection when text is added above.
@MainActor
final class CopyTextHandle {
    weak var view: UITextView?

    private enum Anchor {
        case none
        case bottom
        case select(NSRange)
        /// Text was added above: keep this distance from the bottom, and
        /// move the selection down by the added length.
        case keep(fromBottom: CGFloat, oldLength: Int, selection: NSRange?)
    }
    private var anchor = Anchor.none

    var selectedText: String? {
        guard let view, let range = view.selectedTextRange, !range.isEmpty else { return nil }
        return view.text(in: range)
    }

    func selectAll() {
        guard let view else { return }
        view.becomeFirstResponder()
        view.selectAll(nil)
    }

    /// Scrolls to the end once the next text is set.
    func scrollToBottom() {
        anchor = .bottom
    }

    /// Selects line `index` of `lines` once they are set.
    func selectLine(_ index: Int, of lines: [String]) {
        let start = lines[..<index].reduce(0) { $0 + $1.utf16.count + 1 }
        anchor = .select(NSRange(location: start, length: lines[index].utf16.count))
    }

    /// Runs `change`, which adds text above `old`.
    func keepingPlace(old: [String], _ change: () -> Void) {
        if let view {
            let selection = view.selectedRange.length > 0 ? view.selectedRange : nil
            anchor = .keep(fromBottom: view.contentSize.height - view.contentOffset.y,
                           oldLength: (view.text as NSString).length, selection: selection)
        }
        change()
    }

    /// Called after the view's text changed.
    func textSet() {
        guard let view else { return }
        view.layoutIfNeeded()
        let maxOffset = max(0, view.contentSize.height - view.bounds.height + view.adjustedContentInset.bottom)
        switch anchor {
        case .none:
            return
        case .bottom:
            view.setContentOffset(CGPoint(x: 0, y: maxOffset), animated: false)
        case .select(let range):
            view.becomeFirstResponder()
            view.selectedRange = range
            if let start = view.position(from: view.beginningOfDocument, offset: range.location) {
                let caret = view.caretRect(for: start)
                let y = min(maxOffset, max(0, caret.midY - view.bounds.height / 2))
                view.setContentOffset(CGPoint(x: 0, y: y), animated: false)
            }
        case .keep(let fromBottom, let oldLength, let selection):
            let y = min(maxOffset, max(0, view.contentSize.height - fromBottom))
            view.setContentOffset(CGPoint(x: 0, y: y), animated: false)
            if var selection {
                selection.location += (view.text as NSString).length - oldLength
                view.selectedRange = selection
            }
        }
        anchor = .none
    }
}

/// A read-only, selectable text view in the terminal's font and colors.
struct CopyTextView: UIViewRepresentable {
    let text: String
    let look: TerminalLook
    let handle: CopyTextHandle
    let onNearTop: () -> Void

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = true
        view.alwaysBounceVertical = true
        view.textContainerInset = UIEdgeInsets(top: 8, left: 6, bottom: 8, right: 6)
        view.delegate = context.coordinator
        handle.view = view
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        context.coordinator.onNearTop = onNearTop
        let theme = look.theme
        view.backgroundColor = UIColor(rgb: theme.background)
        view.textColor = UIColor(rgb: theme.foreground)
        view.tintColor = UIColor(rgb: theme.cursor)
        view.font = look.uiFont
        if view.text != text {
            view.text = text
            handle.textSet()
        }
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var onNearTop: () -> Void = {}

        func scrollViewDidScroll(_ scrollView: UIScrollView) {
            // Near the top: read further back. A drag of a selection handle
            // scrolls the view too, so a selection can grow upward.
            if scrollView.contentOffset.y < scrollView.bounds.height {
                onNearTop()
            }
        }
    }
}
