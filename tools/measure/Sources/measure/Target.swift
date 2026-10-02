import AppKit

/// Calibration target: a window whose response to input is known. Each key
/// press or scroll step repaints after `delayMs`, so the harness can be
/// checked against a delay it did not choose.
final class TargetView: NSView {
    var delayMs: Double = 0
    private var presses = 0
    private var offset: CGFloat = 0

    override var acceptsFirstResponder: Bool { true }
    override var isFlipped: Bool { true }

    private func repaint() {
        if delayMs > 0 {
            DispatchQueue.main.asyncAfter(deadline: .now() + delayMs / 1000) { [weak self] in
                self?.needsDisplay = true
            }
        } else {
            needsDisplay = true
        }
    }

    override func keyDown(with event: NSEvent) {
        presses += 1
        repaint()
    }

    override func scrollWheel(with event: NSEvent) {
        offset += event.scrollingDeltaY
        if ProcessInfo.processInfo.environment["MEASURE_TARGET_TRACE"] != nil {
            FileHandle.standardError.write(
                Data("scroll dy=\(event.scrollingDeltaY) offset=\(offset)\n".utf8))
        }
        repaint()
    }

    override func draw(_ dirtyRect: NSRect) {
        (presses % 2 == 0 ? NSColor.black : NSColor.darkGray).setFill()
        bounds.fill()
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedSystemFont(ofSize: 14, weight: .regular),
            .foregroundColor: NSColor.white,
        ]
        let rowHeight: CGFloat = 20
        let first = Int((-offset / rowHeight).rounded(.down))
        var y = -offset.truncatingRemainder(dividingBy: rowHeight) - rowHeight
        var row = first - 1
        while y < bounds.height {
            "row \(row)  presses \(presses)".draw(at: NSPoint(x: 16, y: y), withAttributes: attributes)
            y += rowHeight
            row += 1
        }
    }
}

final class TargetDelegate: NSObject, NSApplicationDelegate {
    let delayMs: Double
    var window: NSWindow?

    init(delayMs: Double) { self.delayMs = delayMs }

    func applicationDidFinishLaunching(_ notification: Notification) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
            styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        window.title = "measure calibration target"
        let view = TargetView(frame: window.contentLayoutRect)
        view.delayMs = delayMs
        window.contentView = view
        window.center()
        window.makeKeyAndOrderFront(nil)
        window.makeFirstResponder(view)
        NSApp.activate(ignoringOtherApps: true)
        self.window = window
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}

func runTarget(delayMs: Double) -> Never {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    let delegate = TargetDelegate(delayMs: delayMs)
    app.delegate = delegate
    app.run()
    exit(0)
}
