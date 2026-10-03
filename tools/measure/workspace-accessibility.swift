// Owned stage04 workspace AX acceptance. Never attaches without checking the
// launch manifest, profile marker, executable digest and exact process args.
// swiftc tools/measure/workspace-accessibility.swift -o /tmp/dbunk-workspace-ax
// /tmp/dbunk-workspace-ax EVIDENCE/identity.json startup|prepare|reopen|quit|dump
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
    var pending = [item]; var result: [AXUIElement] = []
    while let next = pending.popLast() {
        if attribute(next, kAXRoleAttribute) as? String == kAXMenuBarRole || result.contains(where: { CFEqual($0, next) }) { continue }
        require(result.count < 20000, "owned accessibility tree has a bounded node count")
        result.append(next)
        pending.append(contentsOf: (attribute(next, kAXChildrenAttribute) as? [AXUIElement] ?? []).reversed())
    }
    return result
}
func named(_ name: String, _ root: AXUIElement) -> AXUIElement? { all(root).first { title($0) == name } }
func prefix(_ name: String, _ root: AXUIElement) -> AXUIElement? { all(root).first { title($0).hasPrefix(name) } }
func wait(_ label: String, timeout: TimeInterval = 10, _ predicate: () -> Bool) {
    let deadline = Date().addingTimeInterval(timeout)
    repeat { if predicate() { print("PASS: \(label)"); return }; RunLoop.current.run(until: Date().addingTimeInterval(0.05)) } while Date() < deadline
    fail(label)
}
let args = CommandLine.arguments
let raceSteps = ["race-setup", "race-close", "race-reconnect", "race-credentials", "force-saved", "force-reopen", "sqlite-ready", "sqlite-fail-save", "sqlite-retry", "missing-reopen"]
let keychainSteps = ["keychain-create", "keychain-reopen", "encrypted-reopen", "keychain-reset"]
require(args.count == 3 || (args.count == 4 && (["tls", "recovery-oversize", "recovery-export"] + keychainSteps).contains(args[2])), "usage: identity.json STEP [TLS_ROOT, EXPORT_PATH or CREDENTIAL_NAMESPACE]")
let launch = json(args[1]); let step = args[2]
require((["windows", "startup", "prepare", "finish-prepare", "queries", "tls", "reopen", "human-ready", "quit", "close-window", "recovery-quit", "recovery-reset", "recovery-oversize", "recovery-export", "recovery-discard", "recovery-inspect", "dump", "screenshot-startup", "screenshot-connection", "screenshot-workspace"] + keychainSteps + raceSteps).contains(step), "known workspace step")
guard let pid = launch["pid"] as? Int32, let executable = launch["executable"] as? String,
      let digest = launch["executable_sha256"] as? String, let profile = launch["profile"] as? String,
      let instance = launch["fixture_instance"] as? String, let cwd = launch["cwd"] as? String,
      profile == canonical(profile), executable == canonical(executable) else { fail("launch identity fields") }
let marker = json(profile + "/.dbunk-native-stage04")
if keychainSteps.contains(step) {
    require(args.count == 4 && marker["credential_namespace"] as? String == args[3] && UUID(uuidString: args[3]) != nil, "explicitly named credential namespace matches isolated profile")
    require(launch["bundle"] is String, "credential UI acceptance requires the marked packaged app")
}
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
if let bundle = launch["bundle"] as? String {
    require(bundle == canonical(bundle), "owned app bundle is canonical")
    require(executable == bundle + "/Contents/MacOS/dbunk-native", "owned executable belongs to the marked app bundle")
    require(app.bundleIdentifier == "codes.imran.dbunk.native.stage04.preflight", "stage04 preflight bundle identity matches running app")
}
let process = Process(); process.executableURL = URL(fileURLWithPath: "/bin/ps")
process.arguments = ["-p", String(pid), "-o", "command="]
let output = Pipe(); process.standardOutput = output; try process.run(); process.waitUntilExit()
let command = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
require(command.trimmingCharacters(in: .whitespacesAndNewlines) == "\(executable) --workspace-profile \(profile) --fixture-manifest \(cwd)/fixture.json", "exact native workspace launch arguments")
let manifest = json(cwd + "/fixture.json")
require(manifest["instance"] as? String == instance, "current launch manifest matches fixture instance")
let root = AXUIElementCreateApplication(pid); AXUIElementSetMessagingTimeout(root, 2)
require(AXIsProcessTrusted(), "Accessibility permission available")
let session = CGSessionCopyCurrentDictionary() as? [String: Any] ?? [:]
require((session["CGSSessionScreenIsLocked"] as? Bool) != true, "macOS session must be unlocked for owned-window acceptance")
app.activate(); AXUIElementSetAttributeValue(root, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(root, "AXEnhancedUserInterface" as CFString, kCFBooleanTrue)
AXUIElementSetAttributeValue(root, "AXManualAccessibility" as CFString, kCFBooleanTrue)
if step == "windows" {
    print("active=\(app.isActive) hidden=\(app.isHidden) policy=\(app.activationPolicy.rawValue)")
    for item in attribute(root, kAXWindowsAttribute) as? [AXUIElement] ?? [] {
        print("owned AX window title=\(title(item)) minimized=\(attribute(item, kAXMinimizedAttribute) as? Bool ?? false)")
    }
    for item in CGWindowListCopyWindowInfo(.optionAll, kCGNullWindowID) as? [[String: Any]] ?? [] where item[kCGWindowOwnerPID as String] as? Int32 == pid {
        print("owned CG window=\(item[kCGWindowNumber as String] ?? "") name=\(item[kCGWindowName as String] ?? "") layer=\(item[kCGWindowLayer as String] ?? "") onscreen=\(item[kCGWindowIsOnscreen as String] ?? "") bounds=\(item[kCGWindowBounds as String] ?? "")")
    }
    exit(0)
}
var window: AXUIElement?
wait("owned workspace window visible") { window = (attribute(root, kAXWindowsAttribute) as? [AXUIElement])?.first { title($0) == "dbunk Native Workspace" }; return window != nil }
require(title(window!) == "dbunk Native Workspace", "separate native workspace window title")
AXUIElementPerformAction(window!, kAXRaiseAction as CFString)
func keyboardReady() {
    if !app.isActive || attribute(root, kAXFrontmostAttribute) as? Bool != true {
        app.activate(); AXUIElementSetAttributeValue(root, kAXFrontmostAttribute as CFString, kCFBooleanTrue)
        wait("owned application reactivated before keyboard input", timeout: 2) { app.isActive && attribute(root, kAXFrontmostAttribute) as? Bool == true }
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
func press(_ label: String) {
    var control: AXUIElement?
    wait("control available: \(label)") {
        control = all(root).first { title($0) == label && [kAXButtonRole, kAXRadioButtonRole, kAXCheckBoxRole].contains(attribute($0, kAXRoleAttribute) as? String ?? "") }
        return control != nil
    }
    pressElement(control!)
}
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
func saveExport(_ path: String) {
    let url = URL(fileURLWithPath: path)
    require(url.deletingLastPathComponent().path == "/private/tmp", "export acceptance uses the native dialog's canonical temporary directory")
    require(!FileManager.default.fileExists(atPath: path), "export destination is new")
    wait("native save dialog available") { all(root).contains { title($0) == "Save" && attribute($0, kAXRoleAttribute) as? String == kAXButtonRole } }
    var filename: AXUIElement?
    wait("native save filename field available") {
        filename = all(root).first { attribute($0, kAXRoleAttribute) as? String == kAXTextFieldRole && ((attribute($0, kAXIdentifierAttribute) as? String == "saveAsNameTextField") || title($0).hasPrefix("Save As")) }
        return filename != nil
    }
    require(AXUIElementSetAttributeValue(filename!, kAXValueAttribute as CFString, url.lastPathComponent as CFString) == .success, "native save filename accepts its new value")
    wait("native save filename updated") { attribute(filename!, kAXValueAttribute) as? String == url.lastPathComponent }
    press("Save")
    wait("native export file created") { FileManager.default.fileExists(atPath: path) }
}
let draft = "-- native-stage04-ax-α\nSELECT 41 + 1 AS answer;\n"
func capture(_ label: String) {
    let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
    guard let captured = windows.first(where: { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }),
          let windowID = captured[kCGWindowNumber as String] as? Int else { fail("owned visible window capture identity") }
    let path = URL(fileURLWithPath: args[1]).deletingLastPathComponent().appendingPathComponent(label + ".png").path
    let capture = Process(); capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture"); capture.arguments = ["-x", "-l", String(windowID), path]
    do { try capture.run() } catch { fail("window capture could not start") }; capture.waitUntilExit(); require(capture.terminationStatus == 0, "owned window screenshot written")
    print("PASS: screenshot \(path)")
}
if step.hasPrefix("screenshot-") { capture(step)
} else if step == "dump" {
    for item in all(root) { let name = title(item); if !name.isEmpty { print("\(attribute(item, kAXRoleAttribute) as? String ?? "") \(name)") } }
} else if step == "startup" {
    wait("startup distinguishes credential onboarding from workspace") { named("Credential storage", root) != nil || named("Connections", root) != nil }
} else if step == "prepare" || step == "finish-prepare" || step == "queries" {
    if step == "prepare" {
    wait("credential onboarding is ready") { named("Credential storage", root) != nil }
    if named("Credential storage", root) != nil { press("Plain SQLite"); press("Save"); wait("onboarding commits") { named("Credential storage", root) == nil } }
    press("+ Connection"); field("Name", "Stage04 AX fixture"); field("Database password", "dbunk", secret: true)
    key(0, .maskCommand); key(40, .maskControl)
    capture("screenshot-connection")
    press("Test connection"); wait("explicit connection test succeeds") { prefix("Connected in ", root) != nil }
    }
    if step != "queries" {
        press("Save"); wait("connection form closes") { named("PostgreSQL connection", root) == nil }
        press("Stage04 AX fixture"); press("+ Query")
    }
    key(16, .maskControl)
    require(!(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } ?? "").contains("dbunk"), "secure field does not leak through the editor kill ring")
    print("PASS: secure-field kill ring copy is blocked")
    field("SQL editor", draft)
    press("Connect"); wait("first explicit connect reaches Ready") { queryStatus("Ready") }
    key(36, .maskCommand); wait("query result completes with answer 42") { queryStatus("Completed") && hasCell("42") }
    press("+ Query"); field("SQL editor", "SELECT 2 AS second;\n"); press("Connect"); wait("second independent query reaches Ready") { queryStatus("Ready") }
    key(36, .maskCommand); wait("second independent result completes with answer 2") { queryStatus("Completed") && hasCell("2") }
    selectTab("Query 1"); wait("tab switch preserves exact Unicode draft") { named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft }
    require(hasCell("42"), "tab switch preserves its independent result")
    print("PASS: first tab retains its independent result")
    type("-- undo probe"); key(6, .maskCommand)
    wait("tab switch preserves editor undo and focus") { named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft }
    for layout in ["Side by side", "Results first", "Stacked"] {
        press(layout)
        wait("\(layout) preserves editor and result") { hasCell("42") && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft }
    }
    press("Rename"); field("Query name", "AX retained draft"); press("Save"); wait("rename closes") { named("Rename query", root) == nil }
    press("Pin"); press("Pin"); press("←"); selectTab("Query 2"); press("Close")
    wait("closing one tab preserves other document") { prefix("Query 2 ·", root) == nil && prefix("AX retained draft ·", root) != nil }
    wait("draft commit acknowledged") { named("Draft persistence", root).flatMap { attribute($0, kAXValueAttribute) as? String } == "Saved" }
    require(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft, "remaining editor retains exact draft")
    capture("screenshot-workspace")
    print("PASS: actual-window two sessions, query execution, tabs, rename/pin/order/close, durable draft")
} else if step == "tls" {
    require(args.count == 4, "TLS step requires the owned certificate directory")
    let certificates = args[3]
    guard certificates == canonical(certificates), let tls = manifest["tls"] as? [String: Any],
          let tlsInstance = tls["instance"] as? String,
          tlsInstance == launch["tls_fixture_instance"] as? String,
          tls["fixture"] as? String == "dbunk-native-stage04-tls", tls["host"] as? String == "127.0.0.1",
          tls["port"] as? Int == 15433, tls["database"] as? String == "dbunk_tls_demo",
          URL(fileURLWithPath: certificates).lastPathComponent == tlsInstance else { fail("owned TLS endpoint and certificate directory match launch") }
    let tlsMarker = json(certificates + "/.dbunk-native-tls")
    require(tlsMarker["instance"] as? String == tlsInstance && tlsMarker["project"] as? String == "dbunk-native-stage04-tls", "TLS certificate marker identity")
    let hashes = tlsMarker["files_sha256"] as? [String: String] ?? [:]
    for name in ["ca.pem", "untrusted-ca.pem"] {
        let path = certificates + "/" + name
        require(path == canonical(path), "owned TLS public certificate is not symlinked")
        let data = try Data(contentsOf: URL(fileURLWithPath: path))
        require(SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() == hashes[name], "owned TLS public certificate hash matches marker")
    }
    if named("PostgreSQL connection", root) == nil { press("+ Connection") }
    field("Name", "Stage04 AX TLS fixture"); field("Port", "15433"); field("Database", "dbunk_tls_demo")
    field("Database password", "dbunk", secret: true); field("Root certificate", certificates + "/ca.pem"); press("Verify full")
    field("TLS server name", "wrong.dbunk.invalid"); press("Test connection")
    wait("TLS form rejects wrong server name with typed failure") { named("Connection test failed: Tls(HostnameMismatch)", root) != nil }
    field("TLS server name", "127.0.0.1"); field("Root certificate", certificates + "/untrusted-ca.pem"); press("Test connection")
    wait("TLS form rejects untrusted CA with typed failure") { named("Connection test failed: Tls(CertificateUntrusted)", root) != nil }
    field("Root certificate", certificates + "/ca.pem"); press("Test connection")
    wait("TLS form verifies trusted certificate and hostname") { prefix("Connected in ", root) != nil }
    field("Client certificate", certificates + "/client.pem"); field("Client key", certificates + "/missing-client-key.pem"); press("Test connection")
    wait("TLS form rejects missing client key with typed failure") { named("Connection test failed: Tls(InvalidLocalMaterial)", root) != nil }
    field("Client key", certificates + "/client-key.pem"); press("Test connection")
    wait("TLS form accepts owned client certificate and key") { prefix("Connected in ", root) != nil }
    capture("screenshot-tls-form")
    press("Save"); wait("TLS metadata saves") { named("PostgreSQL connection", root) == nil }
    press("Stage04 AX TLS fixture"); press("+ Query"); field("SQL editor", "SELECT CASE WHEN ssl THEN 7 ELSE 0 END AS verified_tls FROM pg_stat_ssl WHERE pid = pg_backend_pid();\n")
    press("Connect"); wait("saved TLS connection opens") { queryStatus("Ready") }
    key(36, .maskCommand); wait("query confirms its backend uses TLS") { queryStatus("Completed") && hasCell("7") }
    capture("screenshot-tls-query"); press("Close")
    wait("TLS document cleanup preserves original query") { prefix("AX retained draft ·", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft }
    print("PASS: actual TLS form validation, trusted save, TLS session query and owned cleanup")
} else if raceSteps.contains(step) {
    let connectionName = "Stage04 AX race fixture"
    let savedDraft = "-- acknowledged force recovery α\nSELECT 42 AS durable_answer;\n"
    let retryDraft = "-- retained through SQLite busy α\nSELECT 43 AS retried_draft;\n"
    func saved() {
        wait("current draft commit acknowledged") { named("Draft persistence", root).flatMap { attribute($0, kAXValueAttribute) as? String } == "Saved" }
    }
    func connect() { press("Connect"); wait("explicit race connection ready") { queryStatus("Ready") } }
    func activeQuery() {
        field("SQL editor", "SELECT pg_sleep(30), 99 AS must_not_reappear;\n")
        key(36, .maskCommand); wait("long query admitted before overlapping lifecycle action") { queryStatus("Running") }
    }
    func recoveredQuery() {
        field("SQL editor", "SELECT 42 AS recovered;\n")
        key(36, .maskCommand)
        wait("replacement execution completes without stale result") { queryStatus("Completed") && hasCell("42") && !hasCell("99") }
        saved()
    }
    if step == "race-setup" {
        wait("fresh race profile onboarding") { named("Credential storage", root) != nil }
        press("Plain SQLite"); press("Save")
        wait("plain SQLite onboarding completes") { named("Credential storage", root) == nil }
        press("+ Connection"); field("Name", connectionName); field("Database password", "dbunk", secret: true)
        press("Save"); wait("race fixture metadata saved") { named("PostgreSQL connection", root) == nil && named(connectionName, root) != nil }
        press(connectionName); press("+ Query"); press("Rename"); field("Query name", "Race draft"); press("Save")
        wait("named race document exists") { prefix("Race draft ·", root) != nil }
        field("SQL editor", "SELECT 42 AS recovered;\n"); saved()
    } else if step == "race-close" {
        wait("base document retained") { prefix("Race draft ·", root) != nil }
        press(connectionName); press("+ Query"); connect(); activeQuery(); press("Close")
        wait("active-query tab close rejoins base document", timeout: 15) { prefix("Race draft ·", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == "SELECT 42 AS recovered;\n" }
        connect(); recoveredQuery()
    } else if step == "race-reconnect" {
        connect(); activeQuery(); press("Disconnect")
        wait("disconnect joins the admitted query", timeout: 15) { queryStatus("Disconnected") }
        connect(); recoveredQuery()
    } else if step == "race-credentials" {
        // Both transitions overlap an admitted long execution. No OS Keychain
        // is involved; every repetition finishes back in plain SQLite mode.
        for encrypted in [true, false] {
            connect(); activeQuery(); press("Credentials")
            wait("credential modes visible over active query") { named("Encrypted SQLite", root) != nil }
            press(encrypted ? "Encrypted SQLite" : "Plain SQLite")
            if encrypted { field("Credential password", "native-stage04-race-only", secret: true) }
            press("Save")
            wait("credential conversion completes", timeout: 30) { named("Credential storage", root) == nil && queryStatus("Disconnected") }
            connect(); recoveredQuery()
        }
    } else if step == "sqlite-ready" {
        wait("SQLite failure case starts from acknowledged disconnected SQL") { prefix("Race draft ·", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == savedDraft && queryStatus("Disconnected") }
        saved()
    } else if step == "sqlite-fail-save" {
        // The runner holds a reversible BEGIN IMMEDIATE lock only on this
        // fresh profile, until this step observes the failure and returns.
        field("SQL editor", retryDraft)
        wait("SQLite write contention reports failed save", timeout: 45) { named("Draft persistence", root).flatMap { attribute($0, kAXValueAttribute) as? String }?.contains("Workspace storage failed") == true }
        key(12, .maskCommand)
        wait("failed SQLite flush blocks ordinary quit and retains exact edits", timeout: 20) { !app.isTerminated && named("Workspace storage failed; retry before closing", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == retryDraft }
        capture("screenshot-sqlite-failed-save")
        print("PASS: failure retains SQL and blocks close; runner must release its SQLite lock before Retry")
    } else if step == "sqlite-retry" {
        require(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == retryDraft, "failed draft remains exact before Retry")
        press("Retry"); saved()
        require(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == retryDraft, "successful Retry preserves exact current SQL")
        key(12, .maskCommand); wait("retried workspace quits normally", timeout: 15) { app.isTerminated }
    } else if step == "missing-reopen" {
        wait("missing binding restores exact SQL disconnected") { prefix("Race draft ·", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == retryDraft && queryStatus("Disconnected") }
        require(named(connectionName, root) != nil, "unrelated saved connection metadata remains visible")
        press("Connect")
        wait("missing binding produces explicit failure without selecting another connection") { queryStatus("Disconnected") && queryStatus("ConnectionLost") }
        require(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == retryDraft, "missing connection preserves the exact draft")
        require(!hasCell("43"), "missing binding never replays SQL against another connection")
        capture("screenshot-missing-binding")
        key(12, .maskCommand); wait("missing-binding workspace quits normally", timeout: 15) { app.isTerminated }
    } else if step == "force-saved" {
        field("SQL editor", savedDraft); saved()
        require(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == savedDraft, "acknowledged force-termination draft exact")
        capture("screenshot-before-forced-termination")
        print("PASS: exact draft acknowledged; runner may terminate only this verified owned PID")
    } else {
        wait("forced-process restart restores exact acknowledged SQL disconnected") { prefix("Race draft ·", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == savedDraft && queryStatus("Disconnected") }
        capture("screenshot-forced-reopen")
        key(12, .maskCommand); wait("forced-recovery reopen quits normally", timeout: 15) { app.isTerminated }
    }
} else if keychainSteps.contains(step) {
    let connectionName = "Stage04 AX Keychain fixture"
    let credentialDraft = "-- native stage04 scoped credentials\nSELECT 41 + 1 AS answer;\n"
    let phrase = "native-stage04-ax-" + args[3]
    func retained() -> Bool {
        named(connectionName, root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == credentialDraft
    }
    func finishCredentials() {
        wait("credential operation completes", timeout: 30) { named("Credential storage", root) == nil }
    }
    if step == "keychain-create" {
        wait("fresh credential onboarding visible") { named("Credential storage", root) != nil }
        press("Keychain"); press("Save"); finishCredentials()
        press("+ Connection"); field("Name", connectionName); field("Database password", "dbunk", secret: true)
        press("Test connection"); wait("fixture credentials reach PostgreSQL") { prefix("Connected in ", root) != nil }
        press("Save"); wait("scoped Keychain connection saved", timeout: 30) { named("PostgreSQL connection", root) == nil && named(connectionName, root) != nil }
        press(connectionName); press("+ Query"); field("SQL editor", credentialDraft); press("Connect")
        wait("Keychain-backed document connects") { queryStatus("Ready") }
        key(36, .maskCommand); wait("Keychain-backed query returns answer 42") { queryStatus("Completed") && hasCell("42") }
        wait("Keychain query draft acknowledged") { named("Draft persistence", root).flatMap { attribute($0, kAXValueAttribute) as? String } == "Saved" }
        capture("screenshot-keychain-created")
    } else if step == "keychain-reopen" {
        wait("Keychain reopen retains disconnected SQL") { retained() && queryStatus("Disconnected") }
        press("Edit")
        wait("edit never fills a saved secret into the field") { named("Database password", root).flatMap { attribute($0, kAXValueAttribute) as? String } == "" }
        press("Test connection"); wait("blank-password edit test resolves stored Keychain secret") { prefix("Connected in ", root) != nil }
        press("Save"); wait("blank-password save preserves scoped secret") { named("PostgreSQL connection", root) == nil }
        press("Credentials"); wait("credential settings visible") { named("Encrypted SQLite", root) != nil }
        press("Encrypted SQLite"); field("Credential password", phrase, secret: true); press("Save"); finishCredentials()
        require(retained(), "Keychain-to-encrypted transition retains metadata and exact SQL")
        print("PASS: reopened Keychain secret used without display; encrypted transition preserves workspace")
    } else if step == "encrypted-reopen" {
        wait("encrypted profile requires unlock") { named("Unlock", root) != nil }
        field("Credential password", "wrong-" + args[3], secret: true); press("Unlock")
        wait("wrong unlock remains locked with explicit error") { named("Incorrect credential password", root) != nil && named("Unlock", root) != nil }
        field("Credential password", phrase, secret: true); press("Unlock"); finishCredentials()
        require(retained() && queryStatus("Disconnected"), "correct unlock preserves disconnected SQL and metadata")
        press("Edit"); press("Test connection"); wait("encrypted store preserved fixture secret") { prefix("Connected in ", root) != nil }; press("Cancel")
        press("Credentials"); wait("unlocked credential modes available") { named("Keychain", root) != nil }; press("Keychain"); press("Save"); finishCredentials()
        require(retained(), "encrypted-to-Keychain transition retains workspace")
        print("PASS: wrong/correct unlock and conversion back to scoped Keychain")
    } else {
        wait("final Keychain reopen retains workspace") { retained() && queryStatus("Disconnected") }
        press("Credentials"); press("Reset saved passwords")
        wait("credential reset needs explicit password-loss confirmation") { named("Confirm password loss", root) != nil }
        press("Confirm password loss")
        wait("credential reset returns to onboarding", timeout: 30) { named("Credential storage", root) != nil && named("Reset saved passwords", root) == nil && named("Plain SQLite", root) != nil }
        require(retained(), "credential reset retains saved SQL and connection metadata")
        press("Cancel"); capture("screenshot-keychain-reset")
        print("PASS: confirmed credential reset keeps connection metadata and exact SQL")
    }
    key(12, .maskCommand); wait("packaged credential workspace joins cleanup", timeout: 30) { app.isTerminated }
} else if step == "recovery-quit" || step == "recovery-reset" {
    wait("saved workspace recovery error visible") { prefix("Saved workspace is unreadable", root) != nil || prefix("Saved workspace version is unsupported", root) != nil }
    require(named("SQL editor", root) == nil, "failed restore creates no editable draft")
    require(named("Stage04 AX fixture", root) != nil, "recovery retains the saved connection navigator")
    if step == "recovery-reset" {
        press("Reset saved workspace"); wait("explicit reset confirmation visible") { named("Reset saved drafts", root) != nil }
        press("Reset saved drafts")
        wait("explicit reset restores an empty workspace") { named("Reset saved workspace", root) == nil && named("Reset saved drafts", root) == nil && named("Stage04 AX fixture", root) != nil }
        require(named("SQL editor", root) == nil, "reset does not invent or run a draft")
    }
    key(12, .maskCommand); wait("recovery workspace quits normally", timeout: 15) { app.isTerminated }
} else if step == "recovery-export" {
    require(args.count == 4, "recovery export requires a new destination")
    guard let currentSQL = named("SQL editor", root).flatMap({ attribute($0, kAXValueAttribute) as? String }) else { fail("current recovery SQL available") }
    require(currentSQL.utf8.count > 448 * 1024, "recovery export retains an oversized in-memory draft")
    press("Export SQL"); saveExport(args[3])
    let exported = try String(contentsOfFile: args[3], encoding: .utf8)
    require(exported == "-- Query document 1\n\n" + currentSQL + "\n\n", "native export retains every byte of current oversized SQL")
    print("PASS: native export retains exact oversized in-memory SQL")
    press("Discard and quit"); wait("discard requires explicit confirmation") { (named("Unsaved drafts", root) != nil || named("Unsaved SQL", root) != nil) }
    press(named("Discard unsaved drafts and close", root) != nil ? "Discard unsaved drafts and close" : "Discard unsaved SQL and close"); wait("explicit discard joins cleanup", timeout: 15) { app.isTerminated }
} else if step == "recovery-inspect" {
    guard let sql = named("SQL editor", root).flatMap({ attribute($0, kAXValueAttribute) as? String }) else { fail("recovery SQL available") }
    print("Recovery SQL bytes: \(sql.utf8.count); appended marker present: \(sql.contains("-- unsaved recovery tail ")); trailing z count: \(sql.reversed().prefix { $0 == "z" }.count)")
} else if step == "recovery-discard" {
    press("Discard and quit"); wait("discard confirmation visible") { (named("Unsaved drafts", root) != nil || named("Unsaved SQL", root) != nil) }
    press(named("Discard unsaved drafts and close", root) != nil ? "Discard unsaved drafts and close" : "Discard unsaved SQL and close"); wait("explicit discard joins cleanup", timeout: 15) { app.isTerminated }
} else if step == "recovery-oversize" {
    require(args.count == 4, "oversize recovery requires a new export file path")
    wait("near-limit saved draft restored") { prefix("Recovery draft ·", root) != nil && named("SQL editor", root) != nil }
    let editor = named("SQL editor", root)!
    guard let savedSQL = attribute(editor, kAXValueAttribute) as? String else { fail("saved draft accessible text available") }
    require(savedSQL.utf8.count > 440 * 1024, "owned recovery draft is near its storage bound")
    require(AXUIElementSetAttributeValue(editor, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "recovery editor accepts focus")
    var cursor = CFRange(location: savedSQL.utf16.count, length: 0)
    require(AXUIElementSetAttributeValue(editor, kAXSelectedTextRangeAttribute as CFString, AXValueCreate(.cfRange, &cursor)!) == .success, "recovery caret moves to end of exact draft")
    let currentSQL = savedSQL + "-- unsaved recovery tail " + String(repeating: "z", count: 2048)
    // CGEvent's Unicode payload is small. Keep one edit burst below the draft
    // writer's debounce while sending a bounded payload in each keyboard event.
    let tail = Array(("-- unsaved recovery tail " + String(repeating: "z", count: 2048)).utf16)
    keyboardReady()
    for start in stride(from: 0, to: tail.count, by: 32) {
        let units = Array(tail[start..<min(start + 32, tail.count)])
        for down in [true, false] {
            let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down)!
            event.flags = []; event.keyboardSetUnicodeString(stringLength: units.count, unicodeString: units); event.postToPid(pid)
        }
    }
    wait("oversized SQL remains exact in memory") { attribute(editor, kAXValueAttribute) as? String == currentSQL }
    wait("oversized draft reports failed persistence") { named("Draft persistence", root).flatMap { attribute($0, kAXValueAttribute) as? String }?.contains("byte budget") == true }
    key(12, .maskCommand)
    wait("failed save blocks normal close and keeps current SQL") { !app.isTerminated && named("Workspace exceeds its byte budget; drafts were not saved", root) != nil && attribute(editor, kAXValueAttribute) as? String == currentSQL }
    capture("screenshot-oversized-draft")
    press("Export SQL"); saveExport(args[3])
    let exported = try String(contentsOfFile: args[3], encoding: .utf8)
    require(exported == "-- Query document 1\n\n" + currentSQL + "\n\n", "native export retains every byte of current oversized SQL")
    print("PASS: native export retains exact oversized in-memory SQL")
    press("Discard and quit"); wait("discard requires explicit confirmation") { (named("Unsaved drafts", root) != nil || named("Unsaved SQL", root) != nil) }
    press(named("Discard unsaved drafts and close", root) != nil ? "Discard unsaved drafts and close" : "Discard unsaved SQL and close"); wait("explicit discard joins cleanup", timeout: 15) { app.isTerminated }
} else if step == "human-ready" {
    wait("human test profile restores its acknowledged SQL") { prefix("AX retained draft ·", root) != nil && named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft }
    press("Connect"); wait("human test fixture connects explicitly") { queryStatus("Ready") }
    key(36, .maskCommand); wait("human test has an accessible result 42") { queryStatus("Completed") && hasCell("42") }
    require(AXUIElementSetAttributeValue(named("SQL editor", root)!, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success, "human test starts focused in SQL")
    capture("screenshot-human-ready")
    print("PASS: verified isolated workspace left open; no further foreground automation")
} else if step == "reopen" {
    wait("restored query tab") { prefix("AX retained draft ·", root) != nil }
    require(named("SQL editor", root).flatMap { attribute($0, kAXValueAttribute) as? String } == draft, "reopen restores exact acknowledged Unicode SQL")
    require(queryStatus("Disconnected"), "reopen restores disconnected state without SQL replay")
    print("PASS: restored draft remains disconnected")
} else if step == "quit" {
    key(12, .maskCommand); wait("workspace quit joins cleanup", timeout: 15) { app.isTerminated }
} else if step == "close-window" {
    guard let close = attribute(window!, kAXCloseButtonAttribute) else { fail("owned titlebar close button available") }
    require(CFGetTypeID(close) == AXUIElementGetTypeID(), "titlebar close is an accessibility element")
    // A fast successful shutdown can close the AX connection before its reply.
    AXUIElementPerformAction(unsafeBitCast(close, to: AXUIElement.self), kAXPressAction as CFString)
    wait("titlebar close saves drafts and joins cleanup", timeout: 15) { app.isTerminated }
}
