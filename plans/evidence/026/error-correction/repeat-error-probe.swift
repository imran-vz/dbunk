// Reduced foreground AX reproduction, not the full Plan 026 workflow.
// Focused end-to-end check against the isolated GPUI spike, never the daily driver.
// swiftc tools/measure/editor-accessibility.swift -o /tmp/dbunk-editor-ax
// /tmp/dbunk-editor-ax <spike-pid>
// /tmp/dbunk-editor-ax --native-fixture <native-pid> <marked-profile> [--prepare-metrics | --prepare-error-review]
// --prepare-error-review leaves the verified fixture window open for visual/listening review.
import AppKit
import ApplicationServices
import Darwin
import Foundation

func fail(_ message: String) -> Never {
  fputs("FAIL: \(message)\n", stderr)
  exit(1)
}

func require(_ condition: Bool, _ message: String) {
  if !condition { fail(message) }
}

// Foundation rewrites /private/var to /var on macOS even though POSIX realpath,
// Python and the backend correctly identify /private/var as canonical.
func canonicalPath(_ path: String) -> String? {
  guard let resolved = realpath(path, nil) else { return nil }
  defer { free(resolved) }
  return String(cString: resolved)
}

func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
  var result: CFTypeRef?
  guard AXUIElementCopyAttributeValue(element, name as CFString, &result) == .success else {
    return nil
  }
  return result
}

func named(_ name: String, in element: AXUIElement) -> AXUIElement? {
  if (attribute(element, kAXTitleAttribute) as? String
    ?? attribute(element, kAXDescriptionAttribute) as? String) == name
  {
    return element
  }
  for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] {
    if let found = named(name, in: child) { return found }
  }
  return nil
}

func selectedRange(_ element: AXUIElement) -> CFRange? {
  guard let value = attribute(element, kAXSelectedTextRangeAttribute),
    CFGetTypeID(value) == AXValueGetTypeID()
  else { return nil }
  var range = CFRange()
  guard AXValueGetValue(value as! AXValue, .cfRange, &range) else { return nil }
  return range
}

func parameter(_ element: AXUIElement, _ name: String, _ argument: CFTypeRef) -> CFTypeRef? {
  var result: CFTypeRef?
  guard
    AXUIElementCopyParameterizedAttributeValue(element, name as CFString, argument, &result)
      == .success
  else { return nil }
  return result
}

var prepareAccessibilityRead: (() -> Void)?

func waitFor(_ message: String, timeout: TimeInterval = 5, onFailure: (() -> Void)? = nil, _ condition: () -> Bool) {
  for _ in 0..<Int(timeout / 0.05) {
    prepareAccessibilityRead?()
    if condition() {
      print("PASS: \(message)")
      return
    }
    RunLoop.current.run(until: Date().addingTimeInterval(0.05))
  }
  onFailure?()
  fail(message)
}

let arguments = CommandLine.arguments
let metricOptions = [
  "--prepare-metrics": "many", "--prepare-metrics=many": "many",
  "--prepare-metrics=wide": "wide", "--prepare-metrics=large": "large",
]
let metricFixture = arguments.count == 5 ? metricOptions[arguments[4]] : nil
let checkStartup = arguments.count == 5 && arguments[4] == "--check-startup"
let prepareErrorReview = arguments.count == 5 && arguments[4] == "--prepare-error-review"
let nativeMode = (arguments.count == 4 || metricFixture != nil || checkStartup || prepareErrorReview)
  && arguments[1] == "--native-fixture"
guard (nativeMode || arguments.count == 2),
  let pid = Int32(arguments[nativeMode ? 2 : 1]), pid > 0
else { fail("expected isolated spike PID or --native-fixture PID PROFILE [--prepare-metrics[=many|wide|large] | --check-startup | --prepare-error-review]") }
var registeredApplication: NSRunningApplication?
waitFor("requested application registers with Cocoa", timeout: 15) {
  registeredApplication = NSRunningApplication(processIdentifier: pid)
  return registeredApplication?.executableURL != nil
    && registeredApplication?.isTerminated == false
}
let app = registeredApplication!
var nativeLaunch: (executable: String, fixtureInstance: String)?
if nativeMode {
  let profile = URL(fileURLWithPath: arguments[3], isDirectory: true)
  require(arguments[3] == canonicalPath(arguments[3]), "profile path is canonical")
  let markerURL = profile.appendingPathComponent(".dbunk-native-stage03")
  let launchURL = profile.appendingPathComponent("launch.json")
  require(
    markerURL.path == canonicalPath(markerURL.path)
      && launchURL.path == canonicalPath(launchURL.path),
    "fixture identity files are not symlinks")
  guard let markerData = try? Data(contentsOf: markerURL),
    let launchData = try? Data(contentsOf: launchURL),
    let marker = (try? JSONSerialization.jsonObject(with: markerData)) as? [String: Any],
    let launch = (try? JSONSerialization.jsonObject(with: launchData)) as? [String: Any],
    let executableURL = app.executableURL,
    let executable = canonicalPath(executableURL.path),
    executableURL.lastPathComponent == "dbunk-native",
    launch["executable"] as? String == executable,
    launch["pid"] as? Int32 == pid,
    let fixtureInstance = launch["fixture_instance"] as? String, UUID(uuidString: fixtureInstance) != nil,
    marker["profile_id"] as? String == launch["profile_id"] as? String,
    let profileID = marker["profile_id"] as? String, UUID(uuidString: profileID) != nil,
    marker["version"] as? Int == 1,
    marker["fixture"] as? String == "dbunk-native-stage03",
    marker["host"] as? String == "127.0.0.1", marker["port"] as? Int == 15432,
    marker["database"] as? String == "dbunk_demo"
  else { fail("native PID, executable and launch-owned fixture marker must match") }
  nativeLaunch = (executable, fixtureInstance)
  let identity = Process()
  identity.executableURL = URL(fileURLWithPath: "/bin/ps")
  identity.arguments = ["-p", String(pid), "-o", "command="]
  let output = Pipe()
  identity.standardOutput = output
  do { try identity.run() } catch { fail("cannot verify native process arguments") }
  identity.waitUntilExit()
  let command = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)?
    .trimmingCharacters(in: .whitespacesAndNewlines)
  require(identity.terminationStatus == 0
    && command == "\(app.executableURL!.path) --profile \(profile.path)",
    "native process was launched with this exact fixture profile")
} else {
  require(app.executableURL?.lastPathComponent == "dbunk-gpui-spike", "expected dbunk-gpui-spike")
}
require(AXIsProcessTrusted(), "Accessibility permission is required")
let root = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(root, 2)
var initialWindow: AXUIElement?
// This is a foreground UI probe. macOS can omit a hidden app's window from AX
// traversal; that is not evidence of missing query state. Reacquire only this
// identity-validated fixture, without changing its editor/control focus.
prepareAccessibilityRead = {
  guard !app.isTerminated && !app.isActive else { return }
  app.activate()
  AXUIElementSetAttributeValue(root, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
  if let window = initialWindow { AXUIElementPerformAction(window, kAXRaiseAction as CFString) }
}
waitFor("validated application exposes its window", timeout: 15) {
  initialWindow = (attribute(root, kAXWindowsAttribute) as? [AXUIElement])?.first
  return initialWindow != nil
}
app.activate()
AXUIElementSetAttributeValue(root, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
AXUIElementPerformAction(initialWindow!, kAXRaiseAction as CFString)
AXUIElementSetAttributeValue(root, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
var found: AXUIElement?
waitFor("SQL editor is exposed") {
  found = named("SQL editor", in: root)
  return found != nil
}
var field = found!
waitFor("SQL text is readable") {
  (attribute(field, kAXValueAttribute) as? String)?.contains("SELECT") == true
}
func keyboardWindowReady() -> Bool {
  guard app.isActive,
    attribute(root, kAXFrontmostAttribute) as? Bool == true,
    let focusedWindow = attribute(root, kAXFocusedWindowAttribute),
    CFGetTypeID(focusedWindow) == AXUIElementGetTypeID()
  else { return false }
  return CFEqual(focusedWindow, initialWindow!)
}

func awaitKeyboardWindow() {
  if nativeMode && !keyboardWindowReady() {
    waitFor("validated native application owns the keyboard window") { keyboardWindowReady() }
  }
}

func key(_ code: CGKeyCode, flags: CGEventFlags = []) {
  awaitKeyboardWindow()
  for down in [true, false] {
    let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!
    event.flags = flags
    event.postToPid(pid)
  }
  Thread.sleep(forTimeInterval: 0.1)
}

func type(_ value: String) {
  awaitKeyboardWindow()
  let units = Array(value.utf16)
  for down in [true, false] {
    let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
    event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units)
    event.postToPid(pid)
  }
}

func setSelection(_ location: Int, _ length: Int, in element: AXUIElement = field) {
  var range = CFRange(location: location, length: length)
  let value = AXValueCreate(.cfRange, &range)!
  require(
    AXUIElementSetAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, value)
      == .success,
    "AX selection action accepted")
  waitFor("AX selection reaches editor at \(location):\(length)") {
    let actual = selectedRange(element)
    return actual?.location == location && actual?.length == length
  }
}


require(AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "SQL focus")
func status() -> String { named("Query status", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } ?? "<missing>" }
func runnable() -> Bool { named("Run", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true }
func errorValue() -> String { named("Query error announcement", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } ?? "<missing>" }
func dumpState() { print("STATE status=\(status()) runnable=\(runnable()) active=\(app.isActive) keyboard=\(keyboardWindowReady()) error=\(errorValue()) SQL=\(attribute(field, kAXValueAttribute) as? String ?? "<missing>")") }
func failed() -> Bool { errorValue().contains("42P01") && errorValue().contains("missing_stage03_table") && runnable() }
let sql = "SELECT 'é😀', pg_sleep(0.75);\nSELECT * FROM missing_stage03_table;"
key(0, flags: .maskCommand); type(sql)
waitFor("probe SQL") { attribute(field, kAXValueAttribute) as? String == sql }
let token = (sql as NSString).range(of: "missing_stage03_table")
setSelection(token.location + 2, 0)
for i in 0..<20 {
  waitFor("run available \(i)", onFailure: dumpState) { runnable() }
  key(36, flags: .maskCommand)
  waitFor("statement failure \(i)", onFailure: dumpState) { failed() }
  key(40, flags: .maskCommand); key(34, flags: .maskCommand); key(53)
  key(36, flags: [.maskCommand, .maskShift])
  waitFor("cleared old failure \(i)", onFailure: dumpState) { named("Query failed", in: root) == nil }
  waitFor("script failure \(i)", onFailure: dumpState) { failed() }
}
let close = attribute(initialWindow!, kAXCloseButtonAttribute) as! AXUIElement
let result = AXUIElementPerformAction(close, kAXPressAction as CFString)
require(result == .success || result == .cannotComplete, "native close")
waitFor("native exit") { app.isTerminated }
print("PASS 20 repeated statement/hover/script error cycles")
