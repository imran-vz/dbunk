import CoreGraphics
import Foundation

/// US ANSI virtual key codes for a-z. The Unicode string is set as well, so
/// the character does not depend on the active keyboard layout.
private let letterKeyCodes: [Character: CGKeyCode] = [
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9,
    "b": 11, "q": 12, "w": 13, "e": 14, "r": 15, "y": 16, "t": 17, "o": 31, "u": 32,
    "i": 34, "p": 35, "l": 37, "j": 38, "k": 40, "n": 45, "m": 46,
]

private let source = CGEventSource(stateID: .combinedSessionState)

/// Posts one key press and release to `pid` only. Nothing reaches whichever
/// application is frontmost, so a run cannot type into another window.
/// Returns the mach time just before the key-down was posted.
@discardableResult
func postKey(_ character: Character, to pid: pid_t) -> UInt64 {
    let code = letterKeyCodes[character] ?? 0
    var units = Array(String(character).utf16)
    guard let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
        let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)
    else { fail("cannot create a key event") }
    down.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
    up.keyboardSetUnicodeString(stringLength: units.count, unicodeString: &units)
    let posted = Clock.now()
    down.postToPid(pid)
    up.postToPid(pid)
    return posted
}

/// Posts a virtual key with modifiers (for example Command-Return) to `pid`.
func postChord(_ code: CGKeyCode, flags: CGEventFlags, to pid: pid_t) {
    guard let down = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: true),
        let up = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: false)
    else { fail("cannot create a key event") }
    // A real arrow, Home or End press carries the function and numeric-pad
    // flags; without them AppKit does not translate the key into a movement
    // command, so the requested modifiers are added to those, not swapped in.
    var flags = flags
    if (115...126).contains(code) {
        flags.insert(.maskSecondaryFn)
        flags.insert(.maskNumericPad)
    }
    down.flags = flags
    up.flags = flags
    down.postToPid(pid)
    usleep(8_000)
    up.postToPid(pid)
}

/// True when the frontmost normal window under `point` belongs to `pid`.
func windowUnder(_ point: CGPoint, belongsTo pid: pid_t) -> Bool {
    let list =
        CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
        as? [[String: Any]] ?? []
    // The list is ordered front to back.
    for entry in list {
        guard let layer = entry[kCGWindowLayer as String] as? Int, layer == 0,
            let rect = entry[kCGWindowBounds as String] as? [String: Any],
            let bounds = CGRect(dictionaryRepresentation: rect as CFDictionary),
            bounds.contains(point)
        else { continue }
        return (entry[kCGWindowOwnerPID as String] as? pid_t) == pid
    }
    return false
}

/// Posts one pixel-precise scroll step where the pointer is.
///
/// Scroll events posted to a process do not reach a WebView's content, so
/// these go through the session event tap like a trackpad's. The window
/// server routes them to whatever is under the pointer; the caller parks the
/// pointer over the target window and checks it is still there.
@discardableResult
func postScroll(deltaY: Int32, deltaX: Int32) -> UInt64 {
    guard
        let event = CGEvent(
            scrollWheelEvent2Source: source, units: .pixel, wheelCount: 2, wheel1: deltaY,
            wheel2: deltaX, wheel3: 0)
    else { fail("cannot create a scroll event") }
    let posted = Clock.now()
    event.post(tap: .cghidEventTap)
    return posted
}

/// Clicks at a screen point, `count` times in quick succession (2 is a
/// double click). Like scrolling, a click is routed by where the pointer is,
/// so it goes through the session event tap; the caller checks the point is
/// over the target window first.
func postClick(at point: CGPoint, count: Int = 1) {
    CGWarpMouseCursorPosition(point)
    CGEvent(
        mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point,
        mouseButton: .left)?
        .post(tap: .cghidEventTap)
    usleep(80_000)
    for click in 1...count {
        for type in [CGEventType.leftMouseDown, .leftMouseUp] {
            guard
                let event = CGEvent(
                    mouseEventSource: source, mouseType: type, mouseCursorPosition: point,
                    mouseButton: .left)
            else { fail("cannot create a mouse event") }
            event.setIntegerValueField(.mouseEventClickState, value: Int64(click))
            event.post(tap: .cghidEventTap)
            usleep(20_000)
        }
    }
}
