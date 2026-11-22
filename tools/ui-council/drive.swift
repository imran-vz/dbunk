// Drives and photographs the native dbunk window for the UI council.
//
//   swiftc tools/ui-council/drive.swift -o tools/ui-council/.build/drive
//   drive shot   --pid P --out FILE.png           window only, no shadow
//   drive place  --pid P --x X --y Y --w W --h H  move and resize the main window
//   drive raise  --pid P
//   drive ax     --pid P [--depth N]              roles, names and window-relative frames
//   drive find   --pid P --name TEXT [--role R]   frames of matching elements
//   drive press  --pid P --name TEXT [--role R] [--nth N]   AXPress (falls back to a click)
//   drive click  --pid P --name TEXT [--role R] [--nth N] [--right]
//   drive clickat --pid P --x X --y Y [--right] [--double]  window-relative points
//   drive key    --pid P --chord cmd+k            modifiers: cmd shift alt ctrl
//   drive type   --pid P --text "select 1"
//   drive scroll --pid P --x X --y Y --dy N
//
// Keys go to the pid only. Clicks are real pointer events, so the window is
// raised first and the pointer must not be needed by anyone else meanwhile.
// Name matching is a case-insensitive substring of AXTitle, AXDescription,
// AXLabel or a string AXValue; prefix the name with "=" for an exact match.
import AppKit
import ImageIO
import ApplicationServices
import CoreGraphics
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write((message + "\n").data(using: .utf8)!)
    exit(1)
}

var args = Array(CommandLine.arguments.dropFirst())
guard !args.isEmpty else { fail("usage: drive <command> --pid P ...") }
let command = args.removeFirst()
var options: [String: String] = [:]
var flags = Set<String>()
var index = 0
while index < args.count {
    let key = args[index]
    guard key.hasPrefix("--") else { fail("unexpected argument \(key)") }
    let name = String(key.dropFirst(2))
    if index + 1 < args.count, !args[index + 1].hasPrefix("--") || Double(args[index + 1]) != nil {
        options[name] = args[index + 1]
        index += 2
    } else {
        flags.insert(name)
        index += 1
    }
}
func opt(_ name: String) -> String {
    guard let value = options[name] else { fail("missing --\(name)") }
    return value
}
func num(_ name: String) -> Double {
    guard let value = Double(opt(name)) else { fail("--\(name) must be a number") }
    return value
}
let pid = pid_t(Int32(options["pid"] ?? "0") ?? 0)
if pid == 0 { fail("missing --pid") }

// MARK: window server

struct WindowInfo { let id: CGWindowID; let bounds: CGRect }

func mainWindow() -> WindowInfo {
    let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
    var best: WindowInfo?
    for entry in list {
        guard let owner = entry[kCGWindowOwnerPID as String] as? Int, owner == Int(pid),
            let layer = entry[kCGWindowLayer as String] as? Int, layer == 0,
            let id = entry[kCGWindowNumber as String] as? Int,
            let rect = entry[kCGWindowBounds as String] as? [String: Any],
            let bounds = CGRect(dictionaryRepresentation: rect as CFDictionary),
            bounds.width > 200, bounds.height > 200
        else { continue }
        if (entry[kCGWindowIsOnscreen as String] as? Bool) != true { continue }
        if best == nil || bounds.width * bounds.height > best!.bounds.width * best!.bounds.height {
            best = WindowInfo(id: CGWindowID(id), bounds: bounds)
        }
    }
    guard let window = best else { fail("no window for pid \(pid)") }
    return window
}

// MARK: accessibility

func attribute(_ element: AXUIElement, _ name: String) -> AnyObject? {
    var value: AnyObject?
    guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
    return value
}
func string(_ element: AXUIElement, _ name: String) -> String? {
    guard let value = attribute(element, name) as? String, !value.isEmpty else { return nil }
    return value
}
func frame(_ element: AXUIElement) -> CGRect? {
    guard let pos = attribute(element, kAXPositionAttribute), let size = attribute(element, kAXSizeAttribute)
    else { return nil }
    var point = CGPoint.zero
    var extent = CGSize.zero
    AXValueGetValue(pos as! AXValue, .cgPoint, &point)
    AXValueGetValue(size as! AXValue, .cgSize, &extent)
    return CGRect(origin: point, size: extent)
}
func names(_ element: AXUIElement) -> [String] {
    var out: [String] = []
    for key in [kAXTitleAttribute, kAXDescriptionAttribute, "AXLabel", kAXValueAttribute] {
        if let value = string(element, key) { out.append(value) }
    }
    return out
}

let app = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(app, 3)
func prime() {
    AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
    AXUIElementSetAttributeValue(app, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
    _ = attribute(app, kAXChildrenAttribute)
    usleep(300_000)
}

struct Node { let element: AXUIElement; let role: String; let names: [String]; let frame: CGRect?; let depth: Int }

func walk(maxDepth: Int = 60, maxNodes: Int = 6000) -> [Node] {
    var nodes: [Node] = []
    func visit(_ element: AXUIElement, _ depth: Int) {
        guard nodes.count < maxNodes else { return }
        let role = string(element, kAXRoleAttribute) ?? "?"
        let sub = string(element, kAXSubroleAttribute)
        // The system menu bar lists the user's recent items; never walk it.
        if role == "AXMenuBar" { return }
        nodes.append(Node(element: element, role: sub.map { "\(role)/\($0)" } ?? role, names: names(element), frame: frame(element), depth: depth))
        guard depth < maxDepth, let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] else { return }
        for child in children { visit(child, depth + 1) }
    }
    visit(app, 0)
    return nodes
}

func matches(_ node: Node, _ query: String, _ role: String?) -> Bool {
    if let role, !node.role.lowercased().contains(role.lowercased()) { return false }
    if query.hasPrefix("=") {
        let exact = String(query.dropFirst()).lowercased()
        return node.names.contains { $0.lowercased() == exact }
    }
    return node.names.contains { $0.lowercased().contains(query.lowercased()) }
}

func target() -> Node {
    prime()
    let query = opt("name")
    let role = options["role"]
    let nth = Int(options["nth"] ?? "0") ?? 0
    var found: [Node] = []
    for attempt in 0..<40 {
        found = walk().filter { matches($0, query, role) && $0.frame.map { $0.width > 0 && $0.height > 0 } ?? false }
        if found.count > nth { break }
        if attempt < 39 { usleep(250_000) }
    }
    guard found.count > nth else { fail("no element named \"\(query)\"\(role.map { " role \($0)" } ?? "") (found \(found.count))") }
    return found[nth]
}

func raise() {
    if let running = NSRunningApplication(processIdentifier: pid) {
        running.activate(options: [.activateAllWindows])
    }
    AXUIElementSetAttributeValue(app, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
    if let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement], let window = windows.first {
        AXUIElementPerformAction(window, kAXRaiseAction as CFString)
    }
    usleep(250_000)
}

// MARK: input

let source = CGEventSource(stateID: .combinedSessionState)

func click(_ point: CGPoint, right: Bool = false, count: Int = 1) {
    raise()
    let down: CGEventType = right ? .rightMouseDown : .leftMouseDown
    let up: CGEventType = right ? .rightMouseUp : .leftMouseUp
    let button: CGMouseButton = right ? .right : .left
    CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: button)?.post(tap: .cghidEventTap)
    usleep(120_000)
    for n in 1...count {
        let d = CGEvent(mouseEventSource: source, mouseType: down, mouseCursorPosition: point, mouseButton: button)!
        let u = CGEvent(mouseEventSource: source, mouseType: up, mouseCursorPosition: point, mouseButton: button)!
        d.setIntegerValueField(.mouseEventClickState, value: Int64(n))
        u.setIntegerValueField(.mouseEventClickState, value: Int64(n))
        d.post(tap: .cghidEventTap)
        usleep(40_000)
        u.post(tap: .cghidEventTap)
        usleep(60_000)
    }
}

let keyCodes: [String: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12,
    "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23,
    "=": 24, "9": 25, "7": 26, "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34,
    "p": 35, "return": 36, "enter": 36, "l": 37, "j": 38, "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43,
    "/": 44, "n": 45, "m": 46, ".": 47, "tab": 48, "space": 49, "`": 50, "delete": 51, "backspace": 51,
    "escape": 53, "esc": 53, "left": 123, "right": 124, "down": 125, "up": 126, "home": 115, "end": 119,
    "pageup": 116, "pagedown": 121, "f2": 120,
]

func chord(_ spec: String) {
    var flags: CGEventFlags = []
    var keyName = ""
    for part in spec.lowercased().split(separator: "+") {
        switch part {
        case "cmd", "command": flags.insert(.maskCommand)
        case "shift": flags.insert(.maskShift)
        case "alt", "opt", "option": flags.insert(.maskAlternate)
        case "ctrl", "control": flags.insert(.maskControl)
        default: keyName = String(part)
        }
    }
    guard let code = keyCodes[keyName] else { fail("unknown key \(keyName)") }
    if (115...126).contains(code) { flags.insert(.maskSecondaryFn) }
    let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true)!
    let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)!
    down.flags = flags
    up.flags = flags
    down.postToPid(pid)
    usleep(10_000)
    up.postToPid(pid)
    usleep(60_000)
}

func typeText(_ text: String) {
    for scalar in text.unicodeScalars {
        if scalar == "\n" { chord("return"); continue }
        var units = Array(String(scalar).utf16)
        let code = keyCodes[String(scalar).lowercased()] ?? 0
        let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true)!
        let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)!
        down.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
        up.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
        down.postToPid(pid)
        up.postToPid(pid)
        usleep(15_000)
    }
}

// MARK: commands

switch command {
case "shot":
    // Only ever keep an image of exactly the app window: if the PNG's size
    // does not match the window bounds (a transition, a different window,
    // anything that could include other screen content), delete and retry.
    let out = opt("out")
    var kept = false
    for attempt in 0..<4 {
        let window = mainWindow()
        try? FileManager.default.removeItem(atPath: out)
        let task = Process()
        task.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        task.arguments = ["-x", "-o", "-l", String(window.id), out]
        try! task.run()
        task.waitUntilExit()
        if task.terminationStatus == 0,
            let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: out) as CFURL, nil),
            let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any],
            let width = props[kCGImagePropertyPixelWidth as String] as? Int,
            let height = props[kCGImagePropertyPixelHeight as String] as? Int
        {
            let scale = Double(width) / Double(window.bounds.width)
            if abs(scale - (Double(height) / Double(window.bounds.height))) < 0.01, scale >= 1, scale <= 3,
                abs(scale - scale.rounded()) < 0.01
            {
                kept = true
                break
            }
        }
        try? FileManager.default.removeItem(atPath: out)
        if attempt < 3 { usleep(500_000) }
    }
    guard kept else { fail("screenshot did not match the window bounds; nothing kept") }
    print(out)
case "place":
    prime()
    guard let windows = attribute(app, kAXWindowsAttribute) as? [AXUIElement], let window = windows.first else {
        fail("no AX window")
    }
    var origin = CGPoint(x: num("x"), y: num("y"))
    var size = CGSize(width: num("w"), height: num("h"))
    AXUIElementSetAttributeValue(window, kAXPositionAttribute as CFString, AXValueCreate(.cgPoint, &origin)!)
    AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, AXValueCreate(.cgSize, &size)!)
    usleep(300_000)
    print(mainWindow().bounds)
case "raise":
    raise()
case "ax", "find":
    prime()
    let origin = mainWindow().bounds.origin
    let depth = Int(options["depth"] ?? "60") ?? 60
    let nodes = walk(maxDepth: depth)
    for node in nodes {
        if command == "find", !matches(node, opt("name"), options["role"]) { continue }
        let rect = node.frame.map {
            " @\(Int($0.minX - origin.x)),\(Int($0.minY - origin.y)) \(Int($0.width))x\(Int($0.height))"
        } ?? ""
        let label = node.names.isEmpty ? "" : " \"" + node.names.joined(separator: " | ").prefix(80) + "\""
        let indent = command == "ax" ? String(repeating: " ", count: node.depth) : ""
        print(indent + node.role + label + rect)
    }
case "press":
    let node = target()
    if AXUIElementPerformAction(node.element, kAXPressAction as CFString) != .success, let rect = node.frame {
        click(CGPoint(x: rect.midX, y: rect.midY))
    }
    usleep(300_000)
case "click":
    let node = target()
    guard let rect = node.frame else { fail("element has no frame") }
    click(CGPoint(x: rect.midX, y: rect.midY), right: flags.contains("right"), count: flags.contains("double") ? 2 : 1)
    usleep(300_000)
case "clickat":
    let origin = mainWindow().bounds.origin
    click(CGPoint(x: origin.x + num("x"), y: origin.y + num("y")), right: flags.contains("right"), count: flags.contains("double") ? 2 : 1)
    usleep(300_000)
case "hover":
    let origin = mainWindow().bounds.origin
    raise()
    let point = CGPoint(x: origin.x + num("x"), y: origin.y + num("y"))
    CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(600_000)
case "key":
    chord(opt("chord"))
    usleep(200_000)
case "type":
    typeText(opt("text"))
    usleep(200_000)
case "scroll":
    let origin = mainWindow().bounds.origin
    raise()
    let point = CGPoint(x: origin.x + num("x"), y: origin.y + num("y"))
    CGEvent(mouseEventSource: source, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    usleep(100_000)
    let event = CGEvent(scrollWheelEvent2Source: source, units: .pixel, wheelCount: 1, wheel1: Int32(num("dy")), wheel2: 0, wheel3: 0)!
    event.post(tap: .cghidEventTap)
    usleep(300_000)
default:
    fail("unknown command \(command)")
}
