import SwiftTerm
import UIKit

/// SwiftTerm's view, telling whether the application tracks the mouse and
/// showing what an input method is still composing.
final class TrackingTerminalView: TerminalView {
    private(set) var mouseTracking = false
    /// SwiftTerm keeps an input method's unfinished text (pinyin, kana)
    /// without drawing it on iOS; this shows it at the cursor until it
    /// commits.
    let composition = UILabel()

    override init(frame: CGRect) {
        super.init(frame: frame)
        composition.isHidden = true
        composition.isUserInteractionEnabled = false
        addSubview(composition)
        // A long press opens the app's copy view instead of SwiftTerm's menu.
        for recognizer in gestureRecognizers ?? [] where recognizer is UILongPressGestureRecognizer {
            recognizer.isEnabled = false
        }
    }

    /// The screen row at `point`, in this view's coordinates.
    func row(at point: CGPoint) -> Int {
        let rows = max(terminalDimensions.rows, 1)
        let height = caretFrame.height > 0 ? caretFrame.height : bounds.height / CGFloat(rows)
        return max(0, min(rows - 1, Int((point.y - bounds.minY) / height)))
    }

    required init?(coder: NSCoder) { fatalError("not used") }

    nonisolated override func mouseModeChanged(source: Terminal) {
        super.mouseModeChanged(source: source)
        let tracking = source.mouseMode != .off
        DispatchQueue.main.async { [weak self] in self?.mouseTracking = tracking }
    }

    /// The text being composed, empty when nothing is.
    var composingText: String {
        markedTextRange.flatMap { text(in: $0) } ?? ""
    }

    /// Shows the composing text over the cursor, kept inside the view.
    /// Called every display frame.
    func refreshComposition() {
        let text = composingText
        guard !text.isEmpty else {
            if !composition.isHidden {
                composition.isHidden = true
                composition.attributedText = nil
            }
            return
        }
        if composition.attributedText?.string != text || composition.font != font {
            composition.attributedText = NSAttributedString(string: text, attributes: [
                .font: font,
                .foregroundColor: nativeForegroundColor,
                .underlineStyle: NSUnderlineStyle.single.rawValue,
            ])
            composition.backgroundColor = nativeBackgroundColor
        }
        let caret = caretFrame
        let size = composition.sizeThatFits(CGSize(width: CGFloat.greatestFiniteMagnitude, height: caret.height))
        let visible = CGRect(origin: contentOffset, size: bounds.size)
        let width = min(size.width, visible.width)
        let x = max(visible.minX, min(caret.minX, visible.maxX - width))
        composition.frame = CGRect(x: x, y: caret.minY, width: width, height: max(caret.height, size.height))
        composition.isHidden = false
        bringSubviewToFront(composition)
    }
}
