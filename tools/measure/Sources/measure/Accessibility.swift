import ApplicationServices
import Foundation

/// What assistive technology can see of a process: every element's role,
/// name and whether it carries a value, as the Accessibility API reports it.
struct AccessibilityDump {
    var lines: [String] = []
    var roleCounts: [String: Int] = [:]
    var named = 0
    var total = 0
    var truncated = false
}

private func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
        return nil
    }
    return value
}

private func text(_ element: AXUIElement, _ name: String) -> String? {
    guard let value = attribute(element, name) as? String, !value.isEmpty else { return nil }
    return value
}

func dumpAccessibility(pid: pid_t, maxDepth: Int, maxElements: Int) -> AccessibilityDump {
    var dump = AccessibilityDump()
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetMessagingTimeout(app, 2)
    // WebKit and other toolkits build their accessibility tree only once an
    // assistive client announces itself, the way VoiceOver does.
    AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    sleep(2)

    func visit(_ element: AXUIElement, depth: Int) {
        if dump.total >= maxElements {
            dump.truncated = true
            return
        }
        dump.total += 1
        let role = text(element, kAXRoleAttribute) ?? "?"
        let subrole = text(element, kAXSubroleAttribute)
        let name =
            text(element, kAXTitleAttribute) ?? text(element, kAXDescriptionAttribute)
            ?? text(element, "AXLabel")
        var value = ""
        if let string = attribute(element, kAXValueAttribute) as? String {
            value = " value[\(string.count) chars]"
        } else if attribute(element, kAXValueAttribute) != nil {
            value = " value"
        }
        let focused = (attribute(element, kAXFocusedAttribute) as? Bool) == true ? " focused" : ""
        dump.roleCounts[role, default: 0] += 1
        if name != nil { dump.named += 1 }
        let label = name.map { " \"\($0.prefix(60))\"" } ?? ""
        let kind = subrole.map { "\(role)/\($0)" } ?? role
        dump.lines.append(String(repeating: "  ", count: depth) + kind + label + value + focused)
        guard depth < maxDepth,
            let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement]
        else { return }
        for child in children { visit(child, depth: depth + 1) }
    }

    // Toolkits that build their tree lazily (AccessKit) start on the first
    // query and deliver it a frame later, so the first walk is thrown away.
    visit(app, depth: 0)
    sleep(2)
    dump = AccessibilityDump()
    visit(app, depth: 0)
    return dump
}

/// Moves and resizes the process's first window, so both hosts are measured
/// at the same size without either needing a resize hook of its own.
func placeWindow(pid: pid_t, origin: CGPoint, size: CGSize) -> Bool {
    let app = AXUIElementCreateApplication(pid)
    guard let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement],
        let window = windows.first
    else { return false }
    var origin = origin
    var size = size
    guard let position = AXValueCreate(.cgPoint, &origin),
        let extent = AXValueCreate(.cgSize, &size)
    else { return false }
    let moved = AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, position)
    let resized = AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, extent)
    return moved == .success && resized == .success
}

/// Brings the process's first window to the front. `NSRunningApplication`
/// activation is cooperative on current macOS and can be declined when
/// another app is active; raising through the Accessibility API is not.
func raiseWindow(pid: pid_t) {
    let app = AXUIElementCreateApplication(pid)
    AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
    if let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement],
        let window = windows.first
    {
        AXUIElementPerformAction(window, kAXRaiseAction as CFString)
    }
}
