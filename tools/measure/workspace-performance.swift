// Fixture-only workspace performance setup and memory cycling. Compile with swiftc.
// Strict guard intentionally matches workspace-accessibility.swift; this tool
// never accepts an arbitrary PID, profile, SQL source or database endpoint.
import AppKit
import ApplicationServices
import CryptoKit
import Darwin
import Foundation

func fail(_ message: String) -> Never { fputs("FAIL: \(message)\n", stderr); exit(1) }
func require(_ ok: Bool, _ message: String) { if !ok { fail(message) } }
func canonical(_ path: String) -> String? {
    guard let value = realpath(path, nil) else { return nil }
    defer { free(value) }; return String(cString: value)
}
func json(_ path: String) -> [String: Any] {
    guard path == canonical(path), let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
          let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { fail("owned JSON is unavailable or symlinked") }
    return object
}
func attribute(_ item: AXUIElement, _ name: String) -> CFTypeRef? {
    var result: CFTypeRef?; guard AXUIElementCopyAttributeValue(item, name as CFString, &result) == .success else { return nil }; return result
}
func title(_ item: AXUIElement) -> String { attribute(item, kAXTitleAttribute) as? String ?? attribute(item, kAXDescriptionAttribute) as? String ?? "" }
func all(_ item: AXUIElement) -> [AXUIElement] {
    if attribute(item, kAXRoleAttribute) as? String == kAXMenuBarRole { return [] }
    return [item] + (attribute(item, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(all)
}
func named(_ name: String, _ root: AXUIElement) -> AXUIElement? { all(root).first { title($0) == name } }
func prefix(_ name: String, _ root: AXUIElement) -> AXUIElement? { all(root).first { title($0).hasPrefix(name) } }
func wait(_ label: String, timeout: TimeInterval = 10, _ predicate: () -> Bool) {
    let deadline = Date().addingTimeInterval(timeout)
    repeat { if predicate() { print("PASS: \(label)"); return }; RunLoop.current.run(until: Date().addingTimeInterval(0.05)) } while Date() < deadline
    fail(label)
}
let args = CommandLine.arguments
require(args.count == 3 || args.count == 4, "usage: identity.json setup-one|setup-four|many|wide|large|typing|cycles|quit [output-or-typing-fixture]")
let launch = json(args[1]); let step = args[2]
require(["setup-one", "setup-four", "many", "wide", "large", "typing", "cycles", "quit"].contains(step), "known performance step")
guard let pid = launch["pid"] as? Int32, let executable = launch["executable"] as? String,
      let digest = launch["executable_sha256"] as? String, let profile = launch["profile"] as? String,
      let instance = launch["fixture_instance"] as? String, let cwd = launch["cwd"] as? String,
      profile == canonical(profile), executable == canonical(executable) else { fail("launch identity fields") }
let marker = json(profile + "/.dbunk-native-stage04")
guard marker["version"] as? Int == 1, marker["path"] as? String == profile,
      let profileID = marker["profile_id"] as? String, UUID(uuidString: profileID) != nil,
      let fixture = marker["fixtures"] as? [String: Any], fixture["instance"] as? String == instance,
      fixture["fixture"] as? String == "dbunk-native-stage03", fixture["host"] as? String == "127.0.0.1",
      fixture["port"] as? Int == 15432, fixture["database"] as? String == "dbunk_demo" else { fail("marked profile and owned fixture identity match") }
let executableFile = try FileHandle(forReadingFrom: URL(fileURLWithPath: executable))
var hasher = SHA256()
while let data = try executableFile.read(upToCount: 1024 * 1024), !data.isEmpty { hasher.update(data: data) }
try executableFile.close()
require(hasher.finalize().map { String(format: "%02x", $0) }.joined() == digest, "running executable matches launch digest")
guard let app = NSRunningApplication(processIdentifier: pid), !app.isTerminated,
      let url = app.executableURL, canonical(url.path) == executable else { fail("PID executable matches owned launch") }
let process = Process(); process.executableURL = URL(fileURLWithPath: "/bin/ps")
process.arguments = ["-p", String(pid), "-o", "command="]
let output = Pipe(); process.standardOutput = output; try process.run(); process.waitUntilExit()
let command = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
require(command.trimmingCharacters(in: .whitespacesAndNewlines) == "\(executable) --workspace-profile \(profile) --fixture-manifest \(cwd)/fixture.json", "exact native workspace launch arguments")
let manifest = json(cwd + "/fixture.json")
require(NSDictionary(dictionary: manifest).isEqual(to: fixture), "current launch manifest exactly matches marked fixture")
require(manifest["tls"] == nil, "performance uses only the original plain fixture workload")
let root = AXUIElementCreateApplication(pid); AXUIElementSetMessagingTimeout(root, 2)
require(AXIsProcessTrusted(), "Accessibility permission available")
app.activate(); AXUIElementSetAttributeValue(root, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(root, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
var window: AXUIElement?
wait("owned workspace window visible") { window = (attribute(root, kAXWindowsAttribute) as? [AXUIElement])?.first; return window != nil }
require(title(window!) == "dbunk Native Workspace", "separate native workspace window title")
AXUIElementPerformAction(window!, kAXRaiseAction as CFString)
func keyboardReady() {
    if !app.isActive || attribute(root, kAXFrontmostAttribute) as? Bool != true {
        app.activate(); AXUIElementSetAttributeValue(root, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
        let deadline = Date().addingTimeInterval(2)
        while (!app.isActive || attribute(root, kAXFrontmostAttribute) as? Bool != true) && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
    }
    require(app.isActive && attribute(root, kAXFrontmostAttribute) as? Bool == true, "owned application is foreground before keyboard input")
}
func key(_ code: CGKeyCode, _ flags: CGEventFlags = []) {
    keyboardReady()
    for down in [true, false] { let event = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: down)!; event.flags = flags; event.postToPid(pid) }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}
func type(_ text: String) {
    keyboardReady(); let utf16 = Array(text.utf16)
    for down in [true, false] { let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!; event.flags = []; event.keyboardSetUnicodeString(stringLength: utf16.count, unicodeString: utf16); event.postToPid(pid) }
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
}
func pressElement(_ item: AXUIElement) { require(AXUIElementPerformAction(item, kAXPressAction as CFString) == .success, "control supports AXPress"); RunLoop.current.run(until: Date().addingTimeInterval(0.1)) }
func press(_ label: String) { var control: AXUIElement?; wait("control available: \(label)") { control = named(label, root); return control != nil }; pressElement(control!) }
func field(_ label: String, _ value: String, secret: Bool = false) {
    var control: AXUIElement?; wait("field available: \(label)") { control = named(label, root); return control != nil }
    require(AXUIElementSetAttributeValue(control!, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "field accepts focus")
    key(0, .maskCommand); type(value)
    if secret { require((attribute(control!, kAXValueAttribute) as? String ?? "") != value, "secure field does not expose its secret") }
    else { wait("field text updated: \(label)") { attribute(control!, kAXValueAttribute) as? String == value } }
}
func queryStatus(_ substring: String) -> Bool { named("Query status", root).flatMap { attribute($0, kAXValueAttribute) as? String }?.contains(substring) == true }
func hasCell(_ value: String) -> Bool { named("Query results", root).flatMap { named(value, $0) }.flatMap { attribute($0, kAXValueAttribute) as? String } == value }
func selectTab(_ name: String) { var tab: AXUIElement?; wait("tab exists: \(name)") { tab = prefix(name + " ·", root); return tab != nil }; pressElement(tab!) }
func runnable() -> Bool { named("Run", root).flatMap { attribute($0, kAXEnabledAttribute) as? Bool } == true }
func rename(_ name: String) { press("Rename"); field("Query name", name); press("Save"); wait("rename committed") { named("Rename query", root) == nil } }
func runFixture(_ name: String) {
    require(["many", "wide", "large"].contains(name), "fixed read-only fixture workload")
    let sql = "SELECT * FROM plan024.fixture_\(name);"
    field("SQL editor", sql)
    wait("Run available before metrics query") { runnable() }
    key(36, [.maskCommand, .maskShift])
    wait("metrics result completed with terminal ACK", timeout: 20) { queryStatus("Completed") && runnable() && named("Query results", root) != nil }
    let labels = all(root).map(title)
    let result = labels.first { $0.hasPrefix("Result 1 · ") } ?? ""
    require(!result.isEmpty, "real result heading exposed")
    let omitted = labels.filter { label in ["omitted", "truncated", "partial"].contains { label.lowercased().contains($0) } }
    require(omitted.isEmpty, "complete metrics fixture has no exposed omissions")
    print("RESULT: \(name) expectedRows=\(name == "large" ? 400 : 10000) observed=\(result)")
}
func newDocument(_ name: String) {
    press("+ Query"); rename(name); press("Connect")
    wait("\(name) connected explicitly") { queryStatus("Ready") }
}
func ownForeground() {
    require(app.isActive && attribute(root, kAXFrontmostAttribute) as? Bool == true, "memory cycle target remained foreground; interruption discards capture")
    let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
    guard let target = windows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }),
          let rect = target[kCGWindowBounds as String] as? [String: Double],
          let x = rect["X"], let y = rect["Y"], let width = rect["Width"], let height = rect["Height"] else { fail("owned foreground window bounds") }
    let center = CGPoint(x: x + width / 2, y: y + height / 2)
    for candidate in windows where (candidate[kCGWindowLayer as String] as? Int) == 0 {
        guard let bounds = candidate[kCGWindowBounds as String] as? [String: Double], let bx = bounds["X"], let by = bounds["Y"], let bw = bounds["Width"], let bh = bounds["Height"] else { continue }
        if CGRect(x: bx, y: by, width: bw, height: bh).contains(center) {
            require(candidate[kCGWindowOwnerPID as String] as? Int32 == pid, "memory cycle target remains unobscured")
            return
        }
    }
    fail("foreground window is not visible")
}
func memorySample() -> [String: Any] {
    var usage = rusage_info_v4()
    let status = withUnsafeMutablePointer(to: &usage) {
        $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { proc_pid_rusage(pid, RUSAGE_INFO_V4, $0) }
    }
    require(status == 0, "owned process memory sample available")
    return ["timestamp": Date().timeIntervalSince1970, "physicalFootprintBytes": usage.ri_phys_footprint,
            "lifetimePeakPhysicalFootprintBytes": usage.ri_lifetime_max_phys_footprint]
}
if step == "setup-one" {
    wait("fresh credential onboarding ready") { named("Credential storage", root) != nil }
    press("Plain SQLite"); press("Save"); wait("plain fixture credential mode committed") { named("Credential storage", root) == nil }
    press("+ Connection"); field("Name", "Performance fixture"); field("Database password", "dbunk", secret: true)
    press("Save"); wait("fixture connection saved") { named("PostgreSQL connection", root) == nil }
    press("Performance fixture"); newDocument("Performance 1"); press("Stacked"); runFixture("many")
} else if step == "setup-four" {
    for index in 2...4 { newDocument("Performance \(index)"); runFixture("many") }
    selectTab("Performance 1"); runFixture("many")
} else if ["many", "wide", "large"].contains(step) {
    selectTab("Performance 1"); runFixture(step)
} else if step == "typing" {
    require(args.count == 4, "shared typing fixture required")
    let path = args[3]
    require(path == canonical(path) && path.hasSuffix("/tools/measure/fixtures/editor-2000.sql"), "fixed shared typing fixture path")
    guard let sql = try? String(contentsOfFile: path, encoding: .utf8), let editor = named("SQL editor", root) else { fail("shared typing fixture and editor available") }
    let saved = (NSPasteboard.general.pasteboardItems ?? []).map { item -> NSPasteboardItem in
        let copy = NSPasteboardItem(); for type in item.types { if let data = item.data(forType: type) { copy.setData(data, forType: type) } }; return copy
    }
    require(AXUIElementSetAttributeValue(editor, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "SQL editor focused")
    key(0, .maskCommand); NSPasteboard.general.clearContents(); NSPasteboard.general.setString(sql, forType: .string); key(9, .maskCommand)
    let deadline = Date().addingTimeInterval(10)
    while attribute(editor, kAXValueAttribute) as? String != sql && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
    let complete = attribute(editor, kAXValueAttribute) as? String == sql
    NSPasteboard.general.clearContents(); NSPasteboard.general.writeObjects(saved)
    require(complete, "complete shared typing document loaded over retained real results")
    key(126, .maskCommand); key(124, .maskCommand)
} else if step == "cycles" {
    require(args.count == 4, "cycle evidence output required")
    for index in (2...4).reversed() { selectTab("Performance \(index)"); press("Close"); wait("extra document closed") { prefix("Performance \(index) ·", root) == nil } }
    selectTab("Performance 1"); runFixture("many")
    var samples = [[String: Any]]()
    func record(_ phase: String, _ cycle: Int) { ownForeground(); var sample = memorySample(); sample["phase"] = phase; sample["cycle"] = cycle; samples.append(sample) }
    func settle(_ phase: String, seconds: Double) {
        let deadline = Date().addingTimeInterval(seconds)
        repeat { record(phase, 0); RunLoop.current.run(until: Date().addingTimeInterval(0.1)) } while Date() < deadline
    }
    settle("before", seconds: 5)
    for cycle in 1...20 {
        record("opening", cycle); newDocument("Memory cycle"); runFixture("large"); record("loaded", cycle)
        press("Close"); wait("cycle document fully closed") { prefix("Memory cycle ·", root) == nil && prefix("Performance 1 ·", root) != nil }
        record("closed", cycle)
    }
    settle("after", seconds: 30)
    let report: [String: Any] = ["pid": pid, "cycles": 20, "workload": "plan024.fixture_large, 400 rows per cycle; baseline retains fixture_many",
        "sampling": "100ms while settling plus before open, after completed query and after each close; OS lifetime peak separately includes the whole process lifetime",
        "samples": samples]
    let data = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
    require(!FileManager.default.fileExists(atPath: args[3]), "cycle output must be new")
    try data.write(to: URL(fileURLWithPath: args[3]), options: .atomic)
    print("PASS: 20 actual-window open/run/close cycles and 30-second settled-memory trace")
} else if step == "quit" {
    key(12, .maskCommand); wait("workspace quit joined", timeout: 15) { app.isTerminated }
}
