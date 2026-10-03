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
let windowSteps = Set(["ready", "run-credit", "run-metadata", "run-stream", "stop", "resume", "replace", "reconnect", "recovery", "disconnected", "settled", "close", "quit", "typing-document"])
let windowStep: String? = arguments.count == 5 && arguments[4].hasPrefix("--window-step=")
  ? String(arguments[4].dropFirst("--window-step=".count)) : nil
if let windowStep { require(windowSteps.contains(windowStep), "known native window step") }
let checkStartup = arguments.count == 5 && arguments[4] == "--check-startup"
let prepareErrorReview = arguments.count == 5 && arguments[4] == "--prepare-error-review"
let nativeMode = (arguments.count == 4 || metricFixture != nil || checkStartup || prepareErrorReview || windowStep != nil)
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
  if let bundlePath = launch["bundle"] as? String {
    let bundleID = "codes.imran.dbunk.native.stage04.preflight"
    require(bundlePath == canonicalPath(bundlePath)
      && launch["bundle_id"] as? String == bundleID
      && app.bundleIdentifier == bundleID
      && app.bundleURL?.path == bundlePath
      && executable == bundlePath + "/Contents/MacOS/dbunk-native",
      "packaged native process has the separate preflight bundle identity")
    // Packaged probes cannot derive repository helpers from the executable.
    // Keep the ordinary CLI fault-injection and performance guards unchanged.
    require(checkStartup || windowStep == "recovery" || windowStep == "quit",
      "packaged probe uses only startup, query and quit actions")
    print("PASS: packaged AX identity \(bundleID) at \(bundlePath)")
  } else {
    require(launch["bundle_id"] == nil && app.bundleIdentifier != "codes.imran.dbunk.native.stage04.preflight",
      "preflight bundle requires launcher-owned bundle identity")
  }
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
if checkStartup {
  waitFor("cold native startup reaches Ready with Run enabled without wake input", timeout: 10) {
    guard let status = named("Query status", in: root), let run = named("Run", in: root) else { return false }
    return attribute(status, kAXValueAttribute) as? String == "Ready"
      && attribute(run, kAXEnabledAttribute) as? Bool == true
  }
  print("PASS: identity-checked cold startup exposes readable SQL and a ready session without editing or focus input")
  exit(0)
}
if nativeMode {
  require(AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    == .success, "native SQL editor accepts explicit focus for probe reruns")
}
waitFor("editor owns accessibility focus") {
  if let current = named("SQL editor", in: root) { field = current }
  return attribute(field, kAXFocusedAttribute) as? Bool == true
}
require(selectedRange(field) != nil, "caret is exposed as AXSelectedTextRange")
print("PASS: caret is exposed")

var notifications = Set<String>()
var observer: AXObserver?
require(
  AXObserverCreate(
    pid,
    { _, _, notification, _ in
      notifications.insert(notification as String)
    }, &observer) == .success, "AX observer can be created")
let textObserver = observer!
for notification in [kAXValueChangedNotification, kAXSelectedTextChangedNotification] {
  require(
    AXObserverAddNotification(textObserver, field, notification as CFString, nil) == .success,
    "editor supports \(notification)")
}
CFRunLoopAddSource(CFRunLoopGetCurrent(), AXObserverGetRunLoopSource(textObserver), .defaultMode)

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

func bounds(_ location: Int, _ length: Int, in element: AXUIElement = field) -> CGRect? {
  var range = CFRange(location: location, length: length)
  guard
    let value = parameter(
      element, kAXBoundsForRangeParameterizedAttribute,
      AXValueCreate(.cfRange, &range)!), CFGetTypeID(value) == AXValueGetTypeID()
  else { return nil }
  var rect = CGRect.zero
  guard AXValueGetValue(value as! AXValue, .cgRect, &rect) else { return nil }
  return rect
}

func visibleBounds(_ location: Int, _ length: Int, in element: AXUIElement = field) -> CGRect {
  var rect: CGRect?
  waitFor("text geometry at \(location):\(length)") {
    rect = bounds(location, length, in: element)
    return rect != nil && rect!.height > 0 && (length == 0 || rect!.width > 0)
  }
  return rect!
}

func frame(_ element: AXUIElement) -> CGRect {
  guard let position = attribute(element, kAXPositionAttribute),
    let size = attribute(element, kAXSizeAttribute),
    CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID()
  else { fail("accessible element has screen bounds") }
  var point = CGPoint.zero
  var extent = CGSize.zero
  require(
    AXValueGetValue(position as! AXValue, .cgPoint, &point)
      && AXValueGetValue(size as! AXValue, .cgSize, &extent), "screen bounds decode")
  return CGRect(origin: point, size: extent)
}

// Small reusable actions retain the full native identity and keyboard checks above.
// The Python race runner waits for backend barriers between these actions.
if let step = windowStep {
  func status() -> String {
    named("Query status", in: root).flatMap { attribute($0, kAXValueAttribute) as? String }?.lowercased() ?? ""
  }
  func runnable() -> Bool {
    named("Run", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
  }
  func setSQL(_ sql: String) {
    key(0, flags: .maskCommand)
    type(sql)
    waitFor("window-step SQL reaches editor") { attribute(field, kAXValueAttribute) as? String == sql }
  }
  switch step {
  case "ready":
    waitFor("window session ready", timeout: 15) { status() == "ready" && runnable() }
  case "run-credit", "run-metadata", "run-stream", "recovery":
    waitFor("window admits execution") { runnable() }
    let sql: String
    switch step {
    case "run-metadata": sql = Array(repeating: "SELECT 1 WHERE false;", count: 20).joined(separator: "\n")
    case "run-stream": sql = "SELECT i, pg_sleep(0.002) FROM generate_series(1, 10000) i;"
    case "recovery": sql = "SELECT 'replacement verified';"
    default: sql = "SELECT i, repeat('x', 100) FROM generate_series(1, 100000) i;"
    }
    setSQL(sql)
    key(36, flags: [.maskCommand, .maskShift])
    if step == "recovery" {
      waitFor("new view completes fresh query and terminal ACK") { status().contains("completed") && runnable() }
      key(97)
      key(124)
      waitFor("only fresh result is exposed") {
        named("Query results", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } == "replacement verified"
      }
    } else {
      waitFor("query running with Run disabled") { status().contains("running") && !runnable() }
    }
  case "stop":
    key(47, flags: .maskCommand)
    key(47, flags: .maskCommand)
    waitFor("repeated Stop accepted while consumer held") { status().contains("stopping") }
  case "resume": key(9, flags: [.maskCommand, .maskControl, .maskAlternate])
  case "replace": key(15, flags: [.maskCommand, .maskControl, .maskAlternate])
  case "reconnect": key(31, flags: [.maskCommand, .maskControl, .maskAlternate])
  case "disconnected":
    waitFor("independent queue failure visible") { status().contains("disconnected") && status().contains("queue full") && !runnable() }
  case "settled":
    waitFor("cancelled query terminal ACK settles", timeout: 15) { status().contains("cancelled") && runnable() }
  case "typing-document":
    guard let identity = nativeLaunch else { fail("native identity missing") }
    var repo = URL(fileURLWithPath: identity.executable)
    for _ in 0..<5 { repo.deleteLastPathComponent() }
    let path = repo.appendingPathComponent("tools/measure/fixtures/editor-2000.sql")
    guard let sql = try? String(contentsOf: path, encoding: .utf8) else { fail("shared typing fixture missing") }
    let clipboard = (NSPasteboard.general.pasteboardItems ?? []).map { item in
      let copy = NSPasteboardItem()
      for type in item.types {
        if let data = item.data(forType: type) { copy.setData(data, forType: type) }
      }
      return copy
    }
    key(0, flags: .maskCommand)
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(sql, forType: .string)
    key(9, flags: .maskCommand)
    let deadline = Date().addingTimeInterval(10)
    while attribute(field, kAXValueAttribute) as? String != sql && Date() < deadline {
      RunLoop.current.run(until: Date().addingTimeInterval(0.05))
    }
    let pasted = attribute(field, kAXValueAttribute) as? String == sql
    NSPasteboard.general.clearContents()
    NSPasteboard.general.writeObjects(clipboard)
    require(pasted, "complete shared typing document reaches editor")
    key(126, flags: .maskCommand)
    key(124, flags: .maskCommand)
    print("PASS: shared 2000-line editor document loaded over retained real results")
  case "close", "quit":
    prepareAccessibilityRead = nil
    let started = DispatchTime.now().uptimeNanoseconds
    if step == "quit" {
      key(12, flags: .maskCommand)
    } else {
      guard let close = attribute(initialWindow!, kAXCloseButtonAttribute), CFGetTypeID(close) == AXUIElementGetTypeID()
      else { fail("native close button missing") }
      let result = AXUIElementPerformAction(close as! AXUIElement, kAXPressAction as CFString)
      require(result == .success || result == .cannotComplete, "close reaches native lifecycle")
    }
    waitFor("window shutdown terminates validated process", timeout: 5) { app.isTerminated }
    print("Shutdown milliseconds: \(Double(DispatchTime.now().uptimeNanoseconds - started) / 1_000_000)")
  default: fail("unhandled window step")
  }
  print("PASS: native window step \(step)")
  exit(0)
}

if let metricFixture {
  let metricSQL = "SELECT * FROM plan024.fixture_\(metricFixture);"
  func metricLabels(_ element: AXUIElement) -> [String] {
    let label = attribute(element, kAXTitleAttribute) as? String
      ?? attribute(element, kAXDescriptionAttribute) as? String
    return (label.map { [$0] } ?? [])
      + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap { metricLabels($0) }
  }
  key(0, flags: .maskCommand)
  type(metricSQL)
  waitFor("metrics SQL reaches the real editor") {
    attribute(field, kAXValueAttribute) as? String == metricSQL
  }
  waitFor("metrics execution is admitted", timeout: 10) {
    named("Run", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
  }
  key(36, flags: [.maskCommand, .maskShift])
  waitFor("metrics query completes with a visible result and terminal ACK", timeout: 10) {
    guard let status = named("Query status", in: root),
      let run = named("Run", in: root) else { return false }
    return (attribute(status, kAXValueAttribute) as? String)?.lowercased().contains("completed") == true
      && attribute(run, kAXEnabledAttribute) as? Bool == true
      && metricLabels(root).contains { $0.hasPrefix("Result 1 · ") }
      && named("Query results", in: root) != nil
  }
  require(attribute(field, kAXValueAttribute) as? String == metricSQL,
    "metrics execution preserves the SQL buffer")
  require(AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    == .success, "metrics preparation restores SQL focus")
  waitFor("metrics SQL editor owns focus") { attribute(field, kAXFocusedAttribute) as? Bool == true }
  let labels = metricLabels(root)
  let resultLabel = labels.first { $0.hasPrefix("Result 1 · ") } ?? "missing result label"
  let omissions = labels.filter {
    let text = $0.lowercased()
    return ["omitted", "truncat", "limit", "partial"].contains { text.contains($0) }
  }
  print("Metrics fixture: \(metricFixture); expected source rows: \(metricFixture == "large" ? 400 : 10000)")
  print("Observed result: \(resultLabel)")
  print("AX omission observations: \(omissions.isEmpty ? "none exposed; consult visible diagnostics and capture" : omissions.joined(separator: "; "))")
  print("PASS: metrics ready with real fixture results, accessibility active and SQL editor focused")
  exit(0)
}

let fixture = "SELECT 'é😀e\u{301}';\nSELECT 2;\n"
key(0, flags: .maskCommand)  // Select all.
type(fixture)
waitFor("keyboard edits publish exact Unicode text", onFailure: {
  if nativeMode {
    let actual = attribute(field, kAXValueAttribute) as? String ?? "<unavailable>"
    let selection = selectedRange(field)
    fputs("Disposable probe SQL at failure: \(String(reflecting: String(actual.prefix(4096))))\n", stderr)
    fputs("Input state: active=\(app.isActive) keyWindow=\(keyboardWindowReady()) editorFocus=\(attribute(field, kAXFocusedAttribute) as? Bool == true) selection=\(String(describing: selection))\n", stderr)
  }
}) {
  attribute(field, kAXValueAttribute) as? String == fixture
}
waitFor("text edits notify assistive clients") {
  notifications.contains(kAXValueChangedNotification)
}
notifications.removeAll()
waitFor("caret uses UTF-16 offsets at trailing newline") {
  selectedRange(field)?.location == fixture.utf16.count && selectedRange(field)?.length == 0
}
require(
  (attribute(field, kAXNumberOfCharactersAttribute) as? NSNumber)?.intValue
    == fixture.utf16.count,
  "character count uses UTF-16")
require(
  (attribute(field, kAXInsertionPointLineNumberAttribute) as? NSNumber)?.intValue == 2,
  "trailing newline caret is on the third line")
guard let line = parameter(field, kAXRangeForLineParameterizedAttribute, NSNumber(value: 1)),
  CFGetTypeID(line) == AXValueGetTypeID()
else { fail("line range is exposed") }
require(
  parameter(field, kAXStringForRangeParameterizedAttribute, line) as? String == "SELECT 2;\n",
  "assistive clients can read an individual line")
print("PASS: character count, caret line and line reading")
setSelection(9, 2)  // The supplementary-plane emoji occupies two UTF-16 units.
require(
  attribute(field, kAXSelectedTextAttribute) as? String == "😀", "selected Unicode text is exact")
print("PASS: selected Unicode text is exact")
// The pinned editor groups edits less than 300 ms apart into one undo step.
// Separate fixture setup from the replacement whose undo is being checked.
Thread.sleep(forTimeInterval: 0.5)
type("X")
let edited = fixture.replacingOccurrences(of: "😀", with: "X")
waitFor("typing replaces the real AX-selected range") {
  attribute(field, kAXValueAttribute) as? String == edited
}
key(6, flags: .maskCommand)  // Undo.
waitFor("undo restores accessible text") {
  attribute(field, kAXValueAttribute) as? String == fixture
}
setSelection(0, 0)
key(124)  // Right.
waitFor("keyboard caret movement updates AX") { selectedRange(field)?.location == 1 }
waitFor("selection changes notify assistive clients") {
  notifications.contains(kAXSelectedTextChangedNotification)
}
key(124, flags: .maskShift)
waitFor("keyboard selection updates AX") {
  selectedRange(field)?.location == 1 && selectedRange(field)?.length == 1
    && attribute(field, kAXSelectedTextAttribute) as? String == "E"
}
setSelection(2, 0)
key(123, flags: .maskShift)  // Select backwards.
waitFor("backward keyboard selection updates AX") {
  selectedRange(field)?.location == 1 && selectedRange(field)?.length == 1
}
key(124, flags: .maskShift)
waitFor("selection direction survives publication") {
  selectedRange(field)?.location == 2 && selectedRange(field)?.length == 0
}
setSelection(11, 0)  // Base letter before the combining mark.
key(124)
waitFor("character boundaries match Zed movement") { selectedRange(field)?.location == 13 }
setSelection(11, 2)
require(
  attribute(field, kAXSelectedTextAttribute) as? String == "e\u{301}",
  "combining sequence stays intact")
print("PASS: combining sequence stays intact")
let firstBounds = visibleBounds(0, 1)
require(frame(field).contains(firstBounds), "SQL highlight is inside the editor on screen")
let secondBounds = visibleBounds(1, 1)
let emojiBounds = visibleBounds(9, 2)
let combiningBounds = visibleBounds(11, 2)
let nextLineBounds = visibleBounds(16, 1)
require(abs(firstBounds.maxX - secondBounds.minX) < 1, "adjacent character bounds meet")
require(emojiBounds.width > firstBounds.width, "emoji uses shaped width")
require(
  abs(combiningBounds.width - firstBounds.width) < 1, "combining sequence has one glyph advance")
require(
  abs(nextLineBounds.minX - firstBounds.minX) < 1
    && abs(nextLineBounds.minY - firstBounds.maxY) < 1, "line bounds follow display layout")
let union = visibleBounds(0, 17)
require(
  union.contains(firstBounds) && union.contains(nextLineBounds), "multiline bounds cover both lines"
)
let trailingCaret = visibleBounds(fixture.utf16.count, 0)
require(
  abs(trailingCaret.minY - firstBounds.minY - 2 * firstBounds.height) < 1,
  "trailing newline caret has its own display row")
print("PASS: shaped Unicode, multiline and trailing-caret geometry")
setSelection(0, 0)
key(48)
waitFor("Tab keeps editor focus and indents SQL") {
  attribute(field, kAXFocusedAttribute) as? Bool == true
    && (attribute(field, kAXValueAttribute) as? String) != fixture
}
key(48, flags: .maskShift)
waitFor("Shift-Tab keeps editor focus and outdents SQL") {
  attribute(field, kAXFocusedAttribute) as? Bool == true
    && attribute(field, kAXValueAttribute) as? String == fixture
}

// Pane navigation uses real key events; checking both the focused node and a
// subsequent edit catches an AX tree that merely claims the right focus.
setSelection(1, 1)
key(97)  // F6.
var results: AXUIElement?
waitFor("F6 moves editor focus to results") {
  results = named("Query results", in: root)
  return results.flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
    && attribute(field, kAXFocusedAttribute) as? Bool == false
}
key(97, flags: .maskShift)
waitFor("Shift-F6 restores editor focus and selection") {
  attribute(field, kAXFocusedAttribute) as? Bool == true
    && selectedRange(field)?.location == 1 && selectedRange(field)?.length == 1
}
key(97)
key(48)  // Tab from results.
waitFor("Tab from results returns to editor") {
  attribute(field, kAXFocusedAttribute) as? Bool == true
}
key(97)
key(48, flags: .maskShift)
waitFor("Shift-Tab from results returns to editor") {
  attribute(field, kAXFocusedAttribute) as? Bool == true
}
key(97)
if nativeMode {
  key(36)
  require(named("Cell editor", in: root) == nil, "native results remain read-only")
  key(100)  // F8 reaches query controls without a VoiceOver modifier chord.
  waitFor("F8 reaches the first enabled query control") {
    let expected = named("Run", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
      ? "Run" : "Reconnect"
    return named(expected, in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
  key(53)  // Escape restores previous content focus (results).
  waitFor("toolbar Escape restores results focus") {
    named("Query results", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
} else {
key(36)  // Enter opens the first result cell.
var cell: AXUIElement?
waitFor("Enter opens an accessible cell editor") {
  cell = named("Cell editor", in: root)
  return cell.flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
}
let cellField = cell!
setSelection(0, 0, in: cellField)
require(
  frame(cellField).contains(visibleBounds(0, 1, in: cellField)),
  "cell highlight is inside the cell editor on screen")
let cellText = attribute(cellField, kAXValueAttribute) as? String
key(48)
waitFor("Tab inside cell editor edits instead of leaving results") {
  attribute(cellField, kAXFocusedAttribute) as? Bool == true
    && (attribute(cellField, kAXValueAttribute) as? String) != cellText
}
key(48, flags: .maskShift)
waitFor("Shift-Tab inside cell editor restores indentation") {
  attribute(cellField, kAXFocusedAttribute) as? Bool == true
    && (attribute(cellField, kAXValueAttribute) as? String) == cellText
}
key(97)
waitFor("F6 leaves cell editor for SQL") { attribute(field, kAXFocusedAttribute) as? Bool == true }
key(97)
waitFor("F6 restores the open cell editor") {
  attribute(cellField, kAXFocusedAttribute) as? Bool == true
}
key(53)  // Escape discards.
waitFor("Escape returns cell focus to results") {
  named("Cell editor", in: root) == nil && attribute(results!, kAXFocusedAttribute) as? Bool == true
}
key(36)
waitFor("cell editor can reopen from keyboard") { named("Cell editor", in: root) != nil }
key(1, flags: .maskCommand)  // Cmd-S stages.
waitFor("staging returns focus to results") {
  named("Cell editor", in: root) == nil && attribute(results!, kAXFocusedAttribute) as? Bool == true
}
}
key(97)
type("X")
waitFor("typing after pane navigation reaches the preserved SQL selection") {
  attribute(field, kAXValueAttribute) as? String
    == fixture.replacingOccurrences(of: "SELECT 'é", with: "SXLECT 'é")
}
key(6, flags: .maskCommand)
waitFor("pane navigation leaves SQL undo intact") {
  attribute(field, kAXValueAttribute) as? String == fixture
}

setSelection(0, fixture.utf16.count)
key(51)  // Backspace.
waitFor("empty editor retains readable value and caret") {
  attribute(field, kAXValueAttribute) as? String == ""
    && selectedRange(field)?.location == 0 && selectedRange(field)?.length == 0
}
_ = visibleBounds(0, 0)
key(6, flags: .maskCommand)
waitFor("undo from empty restores text runs") {
  attribute(field, kAXValueAttribute) as? String == fixture
}

let longFixture = (0..<100).map { "SELECT \($0);\n" }.joined()
key(0, flags: .maskCommand)
type(longFixture)
waitFor("long geometry fixture is loaded") {
  attribute(field, kAXValueAttribute) as? String == longFixture
}
let lastLine = (longFixture as NSString).range(of: "SELECT 99;").location
setSelection(lastLine, 0)
let scrolled = visibleBounds(lastLine, 1)
setSelection(0, 0)
let top = visibleBounds(0, 1)
require(top.minY < scrolled.minY, "autoscroll brings distant text into the viewport")
setSelection(lastLine, 0)
let scrolledAgain = visibleBounds(lastLine, 1)
require(
  scrolledAgain.minY > top.minY && scrolledAgain.minX == top.minX,
  "geometry follows autoscroll back down")
require(
  bounds(0, 1) == nil || bounds(0, 1)?.height == 0,
  "offscreen text does not retain stale highlight bounds")
print("PASS: geometry follows vertical autoscroll")

// Toggle soft wrap through Zed's default keyboard binding, then verify that
// a single logical line gets separate highlight rectangles on display rows.
let wrappedFixture = "-- " + String(repeating: "word ", count: 70)
key(0, flags: .maskCommand)
type(wrappedFixture)
waitFor("wrap fixture is loaded") {
  attribute(field, kAXValueAttribute) as? String == wrappedFixture
}
key(40, flags: .maskCommand)  // Cmd-K, Z.
key(6)
setSelection(0, 0)
let wrapStart = visibleBounds(0, 1)
var wrappedIndex: Int?
for index in 1..<wrappedFixture.utf16.count {
  if let rect = bounds(index, 1), rect.minY > wrapStart.minY + 1 {
    wrappedIndex = index
    break
  }
}
require(wrappedIndex != nil, "soft wrapping exposes a second display row")
let beforeWrap = visibleBounds(wrappedIndex! - 1, 1)
let afterWrap = visibleBounds(wrappedIndex!, 1)
require(
  abs(afterWrap.minY - beforeWrap.maxY) < 1 && afterWrap.minX < beforeWrap.minX,
  "highlight moves to the next display row at a soft wrap")
setSelection(wrappedIndex!, 1)
require(
  attribute(field, kAXSelectedTextAttribute) as? String
    == (wrappedFixture as NSString).substring(with: NSRange(location: wrappedIndex!, length: 1)),
  "wrapped run selection maps to the original buffer")
print("PASS: soft-wrap geometry and selection")
if let window = (attribute(root, kAXWindowsAttribute) as? [AXUIElement])?.first,
  let savedSize = attribute(window, kAXSizeAttribute)
{
  var smaller = CGSize(width: 800, height: 700)
  require(
    AXUIElementSetAttributeValue(
      window, kAXSizeAttribute as CFString,
      AXValueCreate(.cgSize, &smaller)!) == .success, "isolated window can resize")
  waitFor("resizing recalculates soft-wrap highlight geometry") {
    guard let start = bounds(0, 1), let middle = bounds(wrappedIndex! - 1, 1) else { return false }
    return middle.minY > start.minY + start.height / 2
  }
  require(
    AXUIElementSetAttributeValue(window, kAXSizeAttribute as CFString, savedSize) == .success,
    "isolated window size is restored")
  waitFor("restoring window size restores wrap geometry") {
    guard let start = bounds(0, 1), let middle = bounds(wrappedIndex! - 1, 1) else { return false }
    return abs(middle.minY - start.minY) < 1
  }
} else {
  fail("window size is available for resize verification")
}
key(40, flags: .maskCommand)
key(6)  // Restore no wrap.
setSelection(0, 0)
let beforeHorizontalScroll = visibleBounds(wrappedFixture.utf16.count - 1, 1)
setSelection(wrappedFixture.utf16.count - 1, 0)
let afterHorizontalScroll = visibleBounds(wrappedFixture.utf16.count - 1, 1)
require(
  afterHorizontalScroll.minX < beforeHorizontalScroll.minX - 100,
  "horizontal autoscroll updates highlight position")
print("PASS: geometry follows horizontal autoscroll")
key(0, flags: .maskCommand)
type(fixture)
waitFor("geometry checks restore Unicode fixture") {
  attribute(field, kAXValueAttribute) as? String == fixture
}
print("PASS: editor accessibility round trip")

if nativeMode {
  func status() -> String {
    guard let element = named("Query status", in: root) else { return "" }
    return (attribute(element, kAXValueAttribute) as? String ?? "").lowercased()
  }
  func press(_ name: String) {
    var available: AXUIElement?
    waitFor("\(name) becomes enabled") {
      available = named(name, in: root)
      return available.flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
    }
    let control = available!
    let role = attribute(control, kAXRoleAttribute) as? String
    let subrole = attribute(control, kAXSubroleAttribute) as? String
    if ["Stacked", "Side by side", "Results first"].contains(name) {
      require(role == kAXRadioButtonRole, "\(name) exposes a layout radio button")
    } else if name.hasPrefix("Result ") {
      // Pinned accesskit_macos 0.26.3 maps Role::Tab to this role/subrole pair.
      require(role == kAXRadioButtonRole && subrole == "AXTabButton",
        "\(name) exposes a native result tab")
    } else if name.hasPrefix("Notices ") {
      // Role::Button with aria_toggled is a checkbox with AXToggle subrole.
      require(role == kAXCheckBoxRole && subrole == "AXToggle",
        "\(name) exposes a native toggle")
    } else if ["Run", "Run script", "Stop", "Reconnect", "Return to SQL"].contains(name) {
      require(role == kAXButtonRole, "\(name) exposes a command button")
    } else {
      fail("missing explicit role expectation for \(name)")
    }
    require(AXUIElementPerformAction(control, kAXPressAction as CFString) == .success,
      "\(name) supports AXPress")
  }
  // Each cycle begins and ends in Stacked. Wait for pane dimensions, not just
  // unchanged SQL, so highlight assertions run against the newly painted layout.
  func cycleLayouts(_ phase: String, allowedStatuses: [String], retainedCell: String? = nil) {
    let text = attribute(field, kAXValueAttribute) as? String
    guard let selection = selectedRange(field) else { fail("layout cycle has SQL selection") }
    for layout in ["Side by side", "Results first", "Stacked"] {
      let before = frame(field)
      press(layout)
      waitFor("\(phase): \(layout) reflows editor bounds") {
        let after = frame(field)
        switch layout {
        case "Side by side":
          return after.width < before.width * 0.75 && after.height > before.height * 1.5
        case "Results first":
          return after.width > before.width * 1.5 && after.height < before.height * 0.5
        default:
          return abs(after.width - before.width) < 2 && after.height > before.height * 1.5
        }
      }
      require(attribute(field, kAXValueAttribute) as? String == text
        && selectedRange(field)?.location == selection.location
        && selectedRange(field)?.length == selection.length,
        "\(phase): \(layout) preserves SQL and selection")
      require(allowedStatuses.contains { status().contains($0) },
        "\(phase): layout switching preserves execution state")
      require(frame(field).contains(visibleBounds(selection.location, selection.length)),
        "\(phase): \(layout) recomputes accessible text geometry")
      if let retainedCell {
        waitFor("\(phase): \(layout) retains the selected result cell") {
          named("Query results", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } == retainedCell
        }
      }
    }
  }
  func sql(_ value: String) {
    require(AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
      == .success, "SQL editor can regain focus")
    key(0, flags: .maskCommand)
    type(value)
    waitFor("SQL input reaches real editor") { attribute(field, kAXValueAttribute) as? String == value }
  }
  waitFor("fixture session is ready for execution") {
    let current = status()
    let usable = ["ready", "completed", "failed"].contains { current.contains($0) }
    let runEnabled = named("Run", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
    return usable && runEnabled && !current.contains("disconnected") && !current.contains("closing")
  }
  sql("SELECT * FROM plan026.exact_values;")
  press("Run script")
  waitFor("real fixture query completes") { status().contains("completed") }
  let savedClipboard = (NSPasteboard.general.pasteboardItems ?? []).map { item in
    let copy = NSPasteboardItem()
    for type in item.types {
      if let data = item.data(forType: type) { copy.setData(data, forType: type) }
    }
    return copy
  }
  defer {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.writeObjects(savedClipboard)
  }
  require(AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    == .success, "editor focus restored before keyboard result inspection")
  key(97)
  waitFor("query result inspection owns focus") {
    named("Query results", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
  let expectedCells = ["NULL", "", "9223372036854775807", "1234567890.12345678901234567890",
    "é😀e\u{301}", "quoted 'value'; still one string"]
  for (column, expected) in expectedCells.enumerated() {
    key(124)  // Initial arrow selects first cell; subsequent arrows advance.
    waitFor("column \(column + 1) exposes exact retained text") {
      named("Query results", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } == expected
    }
    NSPasteboard.general.clearContents()
    key(8, flags: .maskCommand)
    waitFor("column \(column + 1) copies exact retained text") {
      NSPasteboard.general.string(forType: .string) == expected
    }
  }
  func focusedRetainedCell() -> Bool {
    guard let grid = named("Query results", in: root) else { return false }
    let bounds = frame(grid)
    return attribute(grid, kAXFocusedAttribute) as? Bool == true
      && attribute(grid, kAXValueAttribute) as? String == expectedCells.last
      && bounds.width > 0 && bounds.height > 0
  }
  key(97)
  waitFor("F6 returns from exact results to SQL") {
    attribute(field, kAXFocusedAttribute) as? Bool == true
  }
  key(97)
  waitFor("F6 restores visible results and retained selected cell") { focusedRetainedCell() }
  for attempt in 1...2 {
    key(100)  // F8.
    waitFor("F8 route \(attempt) focuses Run control") {
      named("Run", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
    }
  }
  var reachedLayout = false
  for _ in 0..<12 {
    key(48)
    if named("Stacked", in: root).flatMap({ attribute($0, kAXFocusedAttribute) as? Bool }) == true {
      reachedLayout = true
      break
    }
  }
  require(reachedLayout, "Tab from F8 query controls reaches the layout radio group")
  for layout in ["Side by side", "Results first"] {
    key(48)
    waitFor("Tab reaches \(layout) layout radio") {
      named(layout, in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
    }
  }
  key(53)
  waitFor("repeated toolbar route and Escape restore retained grid focus") { focusedRetainedCell() }
  guard let noticesControl = named("Notices 0", in: root) else { fail("Notices control is exposed") }
  require(AXUIElementSetAttributeValue(noticesControl, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    == .success, "Notices control accepts accessible focus")
  waitFor("Notices control owns focus before activation") {
    attribute(noticesControl, kAXFocusedAttribute) as? Bool == true
  }
  press("Notices 0")
  waitFor("Notices replaces the visible results region") { named("Query results", in: root) == nil }
  key(97)
  waitFor("F6 leaves Notices for visible results and retained selected cell") { focusedRetainedCell() }
  key(97)
  waitFor("F6 after Notices returns to SQL") {
    attribute(field, kAXFocusedAttribute) as? Bool == true
  }
  setSelection(7, 1)
  cycleLayouts("completed query", allowedStatuses: ["completed"])
  let keyboardSQL = "SELECT 41;\nSELECT 42;"
  func runnable() -> Bool {
    named("Run", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
  }
  func hasResultCell(_ value: String) -> Bool {
    guard let grid = named("Query results", in: root), let cell = named(value, in: grid) else { return false }
    return attribute(cell, kAXValueAttribute) as? String == value
  }
  sql(keyboardSQL)
  setSelection(0, 0)
  waitFor("caret execution is admitted after terminal ACK") { runnable() }
  key(36, flags: .maskCommand)
  waitFor("Cmd-Enter executes the caret statement") { status().contains("completed") && hasResultCell("41") }
  require(attribute(field, kAXValueAttribute) as? String == keyboardSQL,
    "Cmd-Enter does not insert a newline into SQL")
  require(attribute(field, kAXFocusedAttribute) as? Bool == true,
    "caret execution preserves editor focus")
  let secondStatement = (keyboardSQL as NSString).range(of: "SELECT 42;")
  setSelection(secondStatement.location, secondStatement.length)
  waitFor("selected execution is admitted after terminal ACK") { runnable() }
  key(36, flags: .maskCommand)
  waitFor("Cmd-Enter executes selected SQL") { status().contains("completed") && hasResultCell("42") }
  require(attribute(field, kAXValueAttribute) as? String == keyboardSQL,
    "selected Cmd-Enter preserves the SQL buffer")
  require(selectedRange(field)?.location == secondStatement.location
    && selectedRange(field)?.length == secondStatement.length,
    "selected execution preserves SQL selection")
  waitFor("script execution is admitted after terminal ACK") { runnable() }
  key(36, flags: [.maskCommand, .maskShift])
  waitFor("Cmd-Shift-Enter executes both statements despite a selection") {
    status().contains("completed") && named("Result 2 · 1 rows", in: root) != nil
  }
  require(attribute(field, kAXValueAttribute) as? String == keyboardSQL,
    "Cmd-Shift-Enter does not insert a newline into SQL")
  press("Result 2 · 1 rows")
  waitFor("script second result contains 42") { hasResultCell("42") }
  press("Result 1 · 1 rows")
  waitFor("script first result contains 41") { hasResultCell("41") }
  // A selection after non-ASCII text exercises SQL-byte to AX-UTF16 mapping
  // without changing the editor geometry contract checked above.
  let errorSQL = "SELECT 'é😀', pg_sleep(0.75);\nSELECT * FROM missing_stage03_table;"
  let errorStatement = (errorSQL as NSString).range(of: "SELECT * FROM missing_stage03_table;")
  let errorToken = (errorSQL as NSString).range(of: "missing_stage03_table")
  let missingTableMessage = "relation \"missing_stage03_table\" does not exist"
  func explicitError() -> AXUIElement? {
    guard let surface = named("Query failed", in: root),
      named(missingTableMessage, in: surface) != nil,
      let announcement = named("Query error announcement", in: root),
      let value = attribute(announcement, kAXValueAttribute) as? String,
      value.contains(missingTableMessage), value.contains("42P01")
    else { return nil }
    return surface
  }
  sql(errorSQL)
  setSelection(errorStatement.location, errorStatement.length)
  waitFor("selected failing query is admitted after terminal ACK") { runnable() }
  key(36, flags: .maskCommand)
  waitFor("SQL failure exposes its full message and PostgreSQL code") { explicitError() != nil }
  let errorSurface = explicitError()!
  let announcement = named("Query error announcement", in: root)!
  require(attribute(announcement, kAXRoleAttribute) as? String == kAXGroupRole
    && attribute(announcement, kAXSubroleAttribute) as? String == "AXApplicationAlert",
    "SQL failure has an explicit application alert announcement")
  require(named("Line 2, column 15", in: errorSurface) != nil,
    "selected SQL error reports its original buffer location after non-ASCII text")
  require(attribute(field, kAXFocusedAttribute) as? Bool == true,
    "error arrival preserves editor focus")
  if let grid = named("Query results", in: root) {
    require(frame(errorSurface).minY >= frame(field).maxY - 2
      && frame(errorSurface).maxY <= frame(grid).minY + 2,
      "Stacked query error sits between the SQL editor and results")
  } else {
    fail("failed query retains the read-only results region")
  }
  waitFor("failed query terminal ACK makes Run available") { runnable() }
  key(100)  // F8 from the editor must override Zed's inherited diagnostic action.
  waitFor("F8 from a failed query reaches Run") {
    named("Run", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
  press("Return to SQL")
  waitFor("Return to SQL restores editor focus") {
    attribute(field, kAXFocusedAttribute) as? Bool == true
  }
  setSelection(errorToken.location + 2, 0)
  let diagnosticBounds = visibleBounds(errorToken.location, errorToken.length)
  key(40, flags: .maskCommand)  // Cmd-K, Cmd-I is the pinned editor's Show Hover chord.
  key(34, flags: .maskCommand)
  require(attribute(field, kAXFocusedAttribute) as? Bool == true
    && attribute(field, kAXValueAttribute) as? String == errorSQL,
    "keyboard diagnostic hover preserves SQL and editor focus")
  require(frame(field).contains(diagnosticBounds), "diagnostic token retains accessible text geometry")
  // The pinned Markdown popover does not expose a distinct AX node. This checks
  // the keyboard route and dismissal, not its visibility or spoken contents.
  if prepareErrorReview {
    key(53)
    let reviewSQL = "SELECT 1;\nSELECT 2;\nSELECT * FROM missing_stage03_table;"
    sql(reviewSQL)
    press("Run script")
    waitFor("human review has two result tabs and an explicit SQL error") {
      explicitError() != nil && runnable()
        && named("Result 1 · 1 rows", in: root) != nil
        && named("Result 2 · 1 rows", in: root) != nil
    }
    let reviewToken = (reviewSQL as NSString).range(of: "missing_stage03_table")
    setSelection(reviewToken.location + 2, 0)
    let reviewBounds = visibleBounds(reviewToken.location, reviewToken.length)
    key(40, flags: .maskCommand)
    key(34, flags: .maskCommand)
    // Allow the pinned delayed hover task to paint before the caller captures it.
    RunLoop.current.run(until: Date().addingTimeInterval(0.75))
    let windowBounds = frame(initialWindow!)
    let review: [String: Any] = [
      "pid": pid, "profile": arguments[3], "window_left_open": true,
      "sql": reviewSQL, "token_utf16_location": reviewToken.location,
      "token_utf16_length": reviewToken.length,
      "token_screen_rect": ["x": reviewBounds.minX, "y": reviewBounds.minY,
        "width": reviewBounds.width, "height": reviewBounds.height],
      "window_screen_rect": ["x": windowBounds.minX, "y": windowBounds.minY,
        "width": windowBounds.width, "height": windowBounds.height],
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: review, options: [.sortedKeys]),
      let json = String(data: data, encoding: .utf8)
    else { fail("error review geometry can be serialized") }
    print("ERROR_REVIEW: \(json)")
    print("PREPARED: verified fixture window remains open with Show Hover requested. Verify actual hover visibility and VoiceOver manually; this is not a completed end-to-end pass.")
    exit(0)
  }
  key(53)
  require(attribute(field, kAXFocusedAttribute) as? Bool == true,
    "Escape after diagnostic hover keeps editor focus")
  waitFor("identical SQL error can be rerun after terminal ACK") { runnable() }
  let previousAnnouncement = named("Query error announcement", in: root)!
  key(36, flags: .maskCommand)
  waitFor("fast identical failure has a new announcement identity") {
    guard let next = named("Query error announcement", in: root) else { return false }
    return explicitError() != nil && !CFEqual(previousAnnouncement, next) && runnable()
  }
  key(36, flags: [.maskCommand, .maskShift])
  waitFor("new execution clears the previous failure before its delayed statement completes") {
    named("Query failed", in: root) == nil
      && named("Query error announcement", in: root) == nil
  }
  waitFor("repeated SQL error remains explicitly accessible", onFailure: {
    print("Repeated-error state: status=\(status()), runnable=\(runnable()), SQL=\(attribute(field, kAXValueAttribute) as? String ?? "<missing>")")
    if let node = named("Query error announcement", in: root) {
      print("Repeated-error announcement: \(attribute(node, kAXValueAttribute) as? String ?? "<missing>")")
    }
  }) { explicitError() != nil && runnable() }
  require(attribute(field, kAXValueAttribute) as? String == errorSQL,
    "repeated error leaves the SQL buffer intact")
  key(36, flags: [.maskCommand, .maskShift])
  waitFor("delayed script starts before editing its source") { status().contains("running") }
  sql("SELECT 42;")
  waitFor("editing SQL removes the stale diagnostic location while retaining execution details") {
    guard let surface = explicitError() else { return false }
    return named("Line 2, column 15", in: surface) == nil
  }
  press("Run script")
  waitFor("session remains usable after SQL error") { status().contains("completed") }
  require(named("Query failed", in: root) == nil, "successful execution has no stale error alert")
  func containsRole(_ role: String, in element: AXUIElement) -> Bool {
    if attribute(element, kAXRoleAttribute) as? String == role { return true }
    return (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).contains {
      containsRole(role, in: $0)
    }
  }
  sql("SELECT 1 AS empty_column WHERE false;")
  press("Run script")
  waitFor("zero-row SELECT preserves column metadata") {
    guard status().contains("completed"), named("Result 1 · 0 rows", in: root) != nil,
      let grid = named("Query results", in: root) else { return false }
    return named("empty_column", in: grid) != nil && !containsRole(kAXRowRole, in: grid)
  }
  sql("SET application_name = 'dbunk_native_fixture';")
  press("Run script")
  waitFor("command-only result completes with zero rows and no invented cells") {
    guard status().contains("completed"), named("Result 1 · 0 rows", in: root) != nil,
      let grid = named("Query results", in: root) else { return false }
    return named("empty_column", in: grid) == nil
      && !containsRole(kAXRowRole, in: grid) && !containsRole(kAXCellRole, in: grid)
  }
  // Outside a transaction PostgreSQL emits a WARNING on the notice channel.
  // This exercises visible notices without bypassing the protected DO policy.
  sql("ROLLBACK;")
  press("Run script")
  waitFor("transaction warning arrives through the server notice channel") {
    status().contains("completed") && named("Notices 1", in: root) != nil
  }
  press("Notices 1")
  waitFor("server warning is exposed in the Notices pane") {
    guard let notices = named("Query notices", in: root) else { return false }
    return named("WARNING: there is no transaction in progress", in: notices) != nil
  }
  sql("DO $$ BEGIN RAISE NOTICE 'native fixture refusal notice'; END $$;")
  press("Run script")
  waitFor("protected policy refuses DO without a confirmation override") {
    status().contains("failed")
      && named("Query error announcement", in: root).flatMap { attribute($0, kAXValueAttribute) as? String }?
        .lowercased().contains("safe mode requires confirmation") == true
      && runnable()
  }
  require(named("Notices 0", in: root) != nil,
    "policy-refused DO does not deliver its server-side notice")
  // Core caps each cell at 1 MiB; 200,000 bytes would not exercise truncation.
  sql("SELECT repeat('x', 2000000) AS oversized;")
  press("Run script")
  waitFor("oversized cell completes with its exact core truncation reason", timeout: 10) {
    guard status().contains("completed"), let diagnostics = named("Query diagnostics", in: root) else { return false }
    return named("cellBytes", in: diagnostics) != nil
  }
  require(attribute(field, kAXFocusedAttribute) as? Bool == true,
    "visible-state checks retain SQL focus before reconnect")
  var disconnectSQL = "SELECT pg_backend_pid() AS pid, backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid();"
  sql(disconnectSQL)
  setSelection(7, 20)
  press("Run script")
  waitFor("native query exposes its own backend identity") {
    guard status().contains("completed"), let grid = named("Query results", in: root) else { return false }
    return named("pid", in: grid) != nil && named("backend_start", in: grid) != nil && runnable()
  }
  key(97)
  waitFor("backend identity result owns keyboard focus") {
    named("Query results", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
  key(124)
  var backendPID = ""
  waitFor("query backend PID is available as exact cell text") {
    backendPID = named("Query results", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } ?? ""
    return Int32(backendPID).map { $0 > 0 } == true
  }
  key(124)
  var backendStart = ""
  waitFor("query backend_start is available as exact cell text") {
    backendStart = named("Query results", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } ?? ""
    return backendStart.contains(":") && backendStart.count >= 19
  }
  key(97)
  waitFor("SQL focus returns before fixture-only fault injection") {
    attribute(field, kAXFocusedAttribute) as? Bool == true
  }
  disconnectSQL = "SELECT * FROM missing_stage03_table;"
  sql(disconnectSQL)
  press("Run script")
  waitFor("connection-loss scenario retains an explicit prior query failure") { explicitError() != nil && runnable() }
  setSelection(7, 20)
  guard let launch = nativeLaunch else { fail("native launch identity is available") }
  let executableURL = URL(fileURLWithPath: launch.executable)
  let suffix = Array(executableURL.pathComponents.suffix(5))
  require(suffix == ["apps", "native", "target", "release", "dbunk-native"]
    || suffix == ["apps", "native", "target", "debug", "dbunk-native"],
    "fixture helper is derived only from a validated repository executable")
  var repository = executableURL
  for _ in 0..<5 { repository.deleteLastPathComponent() }
  let helperURL = repository.appendingPathComponent("tools/native/fixture.py")
  require(helperURL.path == canonicalPath(helperURL.path), "fixture helper path is canonical")
  let fault = Process()
  fault.executableURL = URL(fileURLWithPath: "/usr/bin/env")
  fault.arguments = ["python3", helperURL.path, "terminate-query", "--pid", backendPID,
    "--backend-start", backendStart, "--instance", launch.fixtureInstance]
  let faultOutput = Pipe()
  fault.standardOutput = faultOutput
  fault.standardError = faultOutput
  do { try fault.run() } catch { fail("owned fixture fault helper could not start") }
  waitFor("owned fixture fault helper completes", timeout: 10, onFailure: {
    if fault.isRunning { fault.terminate() }
  }) { !fault.isRunning }
  let faultMessage = String(data: faultOutput.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
  require(fault.terminationStatus == 0, "owned fixture fault injection succeeded: \(faultMessage)")
  print(faultMessage.trimmingCharacters(in: .whitespacesAndNewlines))
  waitFor("externally terminated owned query socket becomes disconnected", timeout: 10) {
    status().contains("disconnected")
  }
  require(attribute(field, kAXValueAttribute) as? String == disconnectSQL
    && selectedRange(field)?.location == 7 && selectedRange(field)?.length == 20,
    "connection loss preserves SQL and selection")
  // Record SQL as the prior content pane before explicitly focusing the
  // transient reconnect control; Ready removes that control from the tree.
  key(100)  // F8.
  waitFor("disconnected F8 route focuses Reconnect and remembers SQL focus") {
    named("Reconnect", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
  guard let reconnectControl = named("Reconnect", in: root) else { fail("Reconnect control is exposed") }
  require(AXUIElementSetAttributeValue(reconnectControl, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    == .success, "Reconnect accepts accessible focus")
  waitFor("Reconnect owns focus before activation") {
    attribute(reconnectControl, kAXFocusedAttribute) as? Bool == true
  }
  press("Reconnect")
  waitFor("explicit reconnect reaches Ready without rerunning retained SQL", timeout: 10) {
    status() == "ready" && runnable()
  }
  require(attribute(field, kAXValueAttribute) as? String == disconnectSQL
    && selectedRange(field)?.location == 7 && selectedRange(field)?.length == 20,
    "reconnect preserves retained SQL and selection")
  require(named("Query failed", in: root) == nil && named("Query error announcement", in: root) == nil,
    "new session state does not resurrect the retired session's error")
  waitFor("reconnect restores SQL focus before its control disappears") {
    attribute(field, kAXFocusedAttribute) as? Bool == true && named("Reconnect", in: root) == nil
  }
  sql("SELECT 43;")
  press("Run script")
  waitFor("fresh session executes after explicit reconnect") {
    status().contains("completed") && hasResultCell("43")
  }
  sql("SELECT i, pg_sleep(0.0005) FROM generate_series(1, 3000) AS i;")
  setSelection(7, 1)
  press("Run script")
  waitFor("real row stream exposes its first cell while running", timeout: 10) {
    guard status().contains("running"), let grid = named("Query results", in: root),
      let cell = named("1", in: grid) else { return false }
    return attribute(cell, kAXValueAttribute) as? String == "1"
  }
  key(97)
  waitFor("streaming result inspection owns focus") {
    named("Query results", in: root).flatMap { attribute($0, kAXFocusedAttribute) as? Bool } == true
  }
  key(124)
  waitFor("streaming inspection selects the first retained cell") {
    named("Query results", in: root).flatMap { attribute($0, kAXValueAttribute) as? String } == "1"
  }
  cycleLayouts("streaming rows", allowedStatuses: ["running", "completed"], retainedCell: "1")
  waitFor("streaming query completes after layout switching", timeout: 12) {
    status().contains("completed")
  }
  key(97)
  waitFor("streaming layout checks restore SQL focus") {
    attribute(field, kAXFocusedAttribute) as? Bool == true
  }
  sql("SELECT pg_sleep(30);")
  setSelection(7, 1)
  press("Run script")
  waitFor("long query starts") { status().contains("running") }
  cycleLayouts("running query", allowedStatuses: ["running"])
  waitFor("Stop is available while the query runs") {
    named("Stop", in: root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true
  }
  require(attribute(field, kAXFocusedAttribute) as? Bool == true,
    "keyboard cancellation starts in the SQL editor")
  key(47, flags: .maskCommand)  // Cmd-Period.
  require(attribute(field, kAXValueAttribute) as? String == "SELECT pg_sleep(30);",
    "Cmd-Period cancels without editing SQL")
  cycleLayouts("cancellation", allowedStatuses: ["stopping", "cancelled"])
  waitFor("cancellation settles") { status().contains("cancelled") }
  waitFor("cancelled execution completes terminal ACK before close scenario") { runnable() }
  sql("SELECT pg_sleep(30);")
  press("Run script")
  waitFor("close scenario has an active query") { status().contains("running") }
  guard let window = (attribute(root, kAXWindowsAttribute) as? [AXUIElement])?.first,
    let close = attribute(window, kAXCloseButtonAttribute), CFGetTypeID(close) == AXUIElementGetTypeID()
  else { fail("native window exposes its close button") }
  prepareAccessibilityRead = nil
  let closeStarted = DispatchTime.now().uptimeNanoseconds
  let closeResult = AXUIElementPerformAction(close as! AXUIElement, kAXPressAction as CFString)
  // The process can finish cleanup before replying to AXPress. Only that
  // transport error is admissible, and actual termination is still required.
  require(closeResult == .success || closeResult == .cannotComplete,
    "close reaches native window lifecycle")
  waitFor("validated native application terminates after window closure") {
    app.isTerminated
  }
  let closeElapsedMilliseconds = Double(DispatchTime.now().uptimeNanoseconds - closeStarted) / 1_000_000
  print(String(format: "Native AXClose-to-termination: %.3f ms", closeElapsedMilliseconds))
  print("PASS: real native fixture query/error/cancel/layout/close workflow")
}
