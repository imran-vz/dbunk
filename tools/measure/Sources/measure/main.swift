import AppKit
import CoreGraphics
import Foundation

let usage = """
measure <command> [options]

  windows   --pid P
  latency   --pid P [--count 500] [--interval-ms 120] [--warmup 20] [--timeout-ms 400] [--out F]
  scroll    --pid P --at fx,fy [--seconds 5] [--delta -40] [--horizontal] [--hz N] [--out F]
  footprint --pid P [--seconds 60] [--interval-ms 1000] [--out F]
  startup   [--seconds 12] [--out F] -- <executable> [args]
  chord     --pid P --key CODE [--command] [--shift] [--option] [--control]
  click     --pid P --at fx,fy [--double]
  place     --pid P --size WxH        move the window and set its size in points
  ax        --pid P [--tree] [--depth 14] [--max 1500] [--out F]
  target    [--delay-ms D]            calibration window with a known response

  --foreground                      discard a capture if the target loses foreground

Keys and clicks are posted to the given process only, never to the frontmost
application. Scroll steps go where the pointer is: the pointer is parked over
the target window for the run, and the run stops if it leaves.
Timing comes from the window server's frame display times.
"""

let arguments = Arguments(Array(CommandLine.arguments.dropFirst(2)))
let command = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ""

func targetPid() -> pid_t {
    guard let pid = pid_t(arguments.required("pid")) else { fail("--pid must be a number") }
    return pid
}

func fraction(_ key: String) -> (Double, Double) {
    let parts = arguments.required(key).split(separator: ",").compactMap { Double($0) }
    guard parts.count == 2 else { fail("--\(key) takes fx,fy as fractions of the window") }
    return (parts[0], parts[1])
}

func screenPoint(in window: WindowInfo, _ at: (Double, Double)) -> CGPoint {
    CGPoint(
        x: window.bounds.minX + window.bounds.width * at.0,
        y: window.bounds.minY + window.bounds.height * at.1)
}

func windowRecord(_ window: WindowInfo) -> [String: Any] {
    ["id": window.id, "widthPoints": window.bounds.width, "heightPoints": window.bounds.height]
}

/// Brings the target forward and waits for its window to be the one drawn.
func prepare(_ pid: pid_t) -> WindowInfo {
    activate(pid)
    raiseWindow(pid: pid)
    guard let window = mainWindow(of: pid, waitSeconds: 5) else { fail("no window for \(pid)") }
    usleep(700_000)
    return window
}

/// A background app can still receive PID-targeted keys but render at a
/// different cadence. Never accept those samples as foreground performance.
func checkForeground(_ pid: pid_t, window: WindowInfo) {
    guard arguments.flag("foreground") else { return }
    let center = CGPoint(x: window.bounds.midX, y: window.bounds.midY)
    guard NSRunningApplication(processIdentifier: pid)?.isActive == true,
        windowUnder(center, belongsTo: pid)
    else { fail("target lost foreground or is obscured; run discarded") }
}

switch command {
case "windows":
    for window in windows(of: targetPid()) {
        print("\(window.id) \(Int(window.bounds.width))x\(Int(window.bounds.height)) onScreen=\(window.onScreen)")
    }

case "latency":
    let pid = targetPid()
    let count = arguments.int("count", 500)
    let warmup = arguments.int("warmup", 20)
    let intervalMs = arguments.double("interval-ms", 120)
    let timeoutMs = arguments.double("timeout-ms", 400)
    let window = prepare(pid)
    checkForeground(pid, window: window)
    let capture = startCapture(.window(window.id))
    usleep(500_000)
    let refreshMs = 1000 / Double(capture.refreshHz)
    let letters = Array("asdfghjkl")
    var latencies: [Double] = []
    var missed = 0
    var unquiet = 0
    var previousFrameTime: UInt64 = 0
    for index in 0..<(warmup + count) {
        let started = Date()
        checkForeground(pid, window: window)
        let posted = postKey(letters[index % letters.count], to: pid)
        // A frame displayed between the previous key's response and this post
        // was not caused by input: something else on screen is animating.
        if index >= warmup, previousFrameTime > 0 {
            let stray = capture.log.snapshot().contains {
                $0.displayTime > previousFrameTime
                    && Clock.millis(from: previousFrameTime, to: $0.displayTime) > 3 * refreshMs
                    && $0.displayTime <= posted
            }
            if stray { unquiet += 1 }
        }
        if let frame = capture.log.firstFrame(after: posted, timeoutMs: timeoutMs) {
            checkForeground(pid, window: window)
            previousFrameTime = frame.displayTime
            if index >= warmup {
                latencies.append(Clock.millis(from: posted, to: frame.displayTime))
            }
        } else if index >= warmup {
            missed += 1
        }
        let remaining = intervalMs / 1000 - Date().timeIntervalSince(started)
        if remaining > 0 { usleep(UInt32(remaining * 1_000_000)) }
    }
    capture.stop()
    emit(
        [
            "metric": "key-to-frame latency (ms)",
            "pid": Int(pid),
            "process": processName(pid),
            "window": windowRecord(window),
            "refreshHz": capture.refreshHz,
            "captureResolutionMs": refreshMs,
            "intervalMs": intervalMs,
            "missed": missed,
            "keysWithStrayFramesBefore": unquiet,
            "summary": summary(latencies),
            "samples": latencies,
        ], to: arguments.string("out"))

case "scroll":
    let pid = targetPid()
    let at = fraction("at")
    let seconds = arguments.double("seconds", 5)
    let delta = Int32(arguments.int("delta", -40))
    let horizontal = arguments.flag("horizontal")
    let window = prepare(pid)
    checkForeground(pid, window: window)
    let capture = startCapture(.window(window.id))
    usleep(500_000)
    let hz = arguments.double("hz", Double(capture.refreshHz))
    let refreshMs = 1000 / Double(capture.refreshHz)
    let point = screenPoint(in: window, at)
    let stepMicros = UInt32(1_000_000 / hz)
    // The pointer is parked over the target for the run and put back after.
    let pointerBefore = CGEvent(source: nil)?.location
    CGWarpMouseCursorPosition(point)
    // Warping does not tell applications the pointer moved. A toolkit that
    // routes scrolling by its own record of the pointer needs the event.
    CGEvent(
        mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point,
        mouseButton: .left)?
        .post(tap: .cghidEventTap)
    usleep(150_000)
    guard windowUnder(point, belongsTo: pid) else {
        capture.stop()
        fail("another window covers the scroll point; nothing was posted")
    }
    let started = Clock.now()
    let deadline = Date().addingTimeInterval(seconds)
    var posted = 0
    var interrupted = false
    while Date() < deadline {
        // Checked a few times a second: if the pointer left the window, or
        // another window came forward, stop instead of scrolling that one.
        if posted % 30 == 0 {
            checkForeground(pid, window: window)
            let pointer = CGEvent(source: nil)?.location ?? point
            if !windowUnder(pointer, belongsTo: pid) {
                interrupted = true
                break
            }
        }
        postScroll(deltaY: horizontal ? 0 : delta, deltaX: horizontal ? delta : 0)
        posted += 1
        usleep(stepMicros)
    }
    usleep(200_000)
    let ended = Clock.now()
    capture.stop()
    if let pointerBefore { CGWarpMouseCursorPosition(pointerBefore) }
    if interrupted { fail("the pointer left the target window; run discarded") }
    // The first and last 250 ms are ramp: the first event has to reach the
    // view, and the tail includes the settle after the last event.
    let frames = capture.log.snapshot().filter {
        Clock.millis(from: started, to: $0.displayTime) > 250
            && $0.displayTime < ended
            && Clock.millis(from: $0.displayTime, to: ended) > 450
    }
    var intervals: [Double] = []
    for (previous, next) in zip(frames, frames.dropFirst()) {
        intervals.append(Clock.millis(from: previous.displayTime, to: next.displayTime))
    }
    let long = intervals.filter { $0 > 1.5 * refreshMs }.count
    let measuredSeconds = intervals.reduce(0, +) / 1000
    emit(
        [
            "metric": "scroll frame interval (ms)",
            "pid": Int(pid),
            "process": processName(pid),
            "window": windowRecord(window),
            "refreshHz": capture.refreshHz,
            "captureResolutionMs": refreshMs,
            "eventsPosted": posted,
            "eventHz": hz,
            "eventsPerSecond": Double(posted) / seconds,
            "deltaPixels": Int(delta),
            "horizontal": horizontal,
            "frames": frames.count,
            "framesPerSecond": measuredSeconds > 0 ? Double(intervals.count) / measuredSeconds : 0,
            "longFrames": long,
            "longFrameShare": intervals.isEmpty ? 0 : Double(long) / Double(intervals.count),
            "summary": summary(intervals),
            "samples": intervals,
        ], to: arguments.string("out"))

case "footprint":
    let pid = targetPid()
    let foregroundWindow = arguments.flag("foreground") ? prepare(pid) : nil
    let seconds = arguments.double("seconds", 60)
    let intervalMs = arguments.double("interval-ms", 1000)
    var totals: [Double] = []
    var firstCpu: [pid_t: UInt64] = [:]
    var lastCpu: [pid_t: UInt64] = [:]
    var members: [String: Double] = [:]
    let started = Clock.now()
    let deadline = Date().addingTimeInterval(seconds)
    repeat {
        if let foregroundWindow { checkForeground(pid, window: foregroundWindow) }
        var total: UInt64 = 0
        members = [:]
        for member in processTree(of: pid) {
            guard let reading = sample(member) else { continue }
            total += reading.footprintBytes
            if firstCpu[member] == nil { firstCpu[member] = reading.cpuNanos }
            lastCpu[member] = reading.cpuNanos
            members["\(processName(member)) (\(member))"] =
                Double(reading.footprintBytes) / 1_048_576
        }
        totals.append(Double(total) / 1_048_576)
        usleep(UInt32(intervalMs * 1000))
    } while Date() < deadline
    if let foregroundWindow { checkForeground(pid, window: foregroundWindow) }
    let wallNanos = Double(Clock.nanos(Clock.now() - started))
    let cpuNanos = lastCpu.reduce(0.0) { $0 + Double($1.value - (firstCpu[$1.key] ?? $1.value)) }
    emit(
        [
            "metric": "process-tree footprint (MiB) and CPU",
            "pid": Int(pid),
            "process": processName(pid),
            "seconds": seconds,
            "footprintMiB": summary(totals),
            "lastSampleByProcessMiB": members,
            "cpuPercentOfOneCore": wallNanos > 0 ? 100 * cpuNanos / wallNanos : 0,
            "cpuMillis": cpuNanos / 1_000_000,
        ], to: arguments.string("out"))

case "startup":
    guard let executable = arguments.rest.first else { fail("startup needs -- <executable>") }
    let seconds = arguments.double("seconds", 12)
    let capture = startCapture(.display)
    usleep(500_000)
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = Array(arguments.rest.dropFirst())
    process.standardOutput = FileHandle.nullDevice
    process.standardError = FileHandle.nullDevice
    let spawned = Clock.now()
    do { try process.run() } catch { fail("cannot launch \(executable): \(error)") }
    let pid = process.processIdentifier
    var windowSeen: UInt64?
    var window: WindowInfo?
    let deadline = Date().addingTimeInterval(seconds)
    while Date() < deadline {
        if windowSeen == nil, let found = windows(of: pid).first(where: { $0.onScreen }) {
            windowSeen = Clock.now()
            window = found
        }
        usleep(windowSeen == nil ? 2_000 : 100_000)
    }
    // The window can move or resize while it restores its geometry.
    if let latest = windows(of: pid).first(where: { $0.onScreen }) { window = latest }
    capture.stop()
    process.terminate()
    guard let window, let windowSeen else { fail("no window appeared within \(seconds) s") }
    let pixels = window.bounds.applying(
        CGAffineTransform(scaleX: capture.scale, y: capture.scale))
    let windowTiles = Double(pixels.width * pixels.height) / Double(tileSize * tileSize)
    /// Share of the window's tiles that changed in this frame.
    func coverage(_ frame: Frame) -> Double {
        Double(frame.changedTiles(in: pixels)) / windowTiles
    }
    let after = capture.log.snapshot().filter { $0.displayTime > spawned }
    let large = after.filter { coverage($0) >= 0.05 }
    emit(
        [
            "metric": "startup (ms from spawn)",
            "executable": executable,
            "window": windowRecord(window),
            "windowOnScreenMs": Clock.millis(from: spawned, to: windowSeen),
            "firstLargePaintMs": large.first.map { Clock.millis(from: spawned, to: $0.displayTime) }
                ?? -1,
            "lastLargePaintMs": large.last.map { Clock.millis(from: spawned, to: $0.displayTime) }
                ?? -1,
            "largePaints": large.count,
            "largePaintTimesMs": large.map { Clock.millis(from: spawned, to: $0.displayTime) },
            "largePaintCoverage": large.map { coverage($0) },
            "framesObserved": after.count,
            "largePaintThreshold": "a frame that changes at least 5% of the window",
        ], to: arguments.string("out"))

case "frames":
    // Diagnostic: what the capture reports for a window nobody is touching.
    let pid = targetPid()
    let window = prepare(pid)
    let capture = startCapture(.window(window.id))
    usleep(500_000)
    let started = Clock.now()
    let before = capture.log.delivered
    usleep(UInt32(arguments.double("seconds", 2) * 1_000_000))
    capture.stop()
    let frames = capture.log.snapshot().filter { $0.displayTime > started }
    emit(
        [
            "framesDelivered": capture.log.delivered - before,
            "framesChanged": frames.count,
            "changedTiles": frames.prefix(40).map { $0.changedTiles },
        ], to: arguments.string("out"))

case "ax":
    // The accessibility tree as an assistive client sees it.
    let pid = targetPid()
    _ = prepare(pid)
    let dump = dumpAccessibility(
        pid: pid, maxDepth: arguments.int("depth", 14), maxElements: arguments.int("max", 1500))
    if arguments.flag("tree") { dump.lines.forEach { print($0) } }
    emit(
        [
            "pid": Int(pid),
            "process": processName(pid),
            "elements": dump.total,
            "elementsWithAName": dump.named,
            "truncated": dump.truncated,
            "roles": dump.roleCounts,
        ], to: arguments.string("out"))

case "place":
    let pid = targetPid()
    _ = prepare(pid)
    let size = arguments.required("size").split(separator: "x").compactMap { Double($0) }
    guard size.count == 2 else { fail("--size takes WIDTHxHEIGHT in points") }
    let placed = placeWindow(
        pid: pid, origin: CGPoint(x: 80, y: 60), size: CGSize(width: size[0], height: size[1]))
    guard placed else { fail("the window refused the new frame") }
    usleep(500_000)
    if let window = mainWindow(of: pid) {
        print("\(Int(window.bounds.width))x\(Int(window.bounds.height))")
    }

case "chord":
    let pid = targetPid()
    guard let code = UInt16(arguments.required("key")) else { fail("--key is a virtual key code") }
    var flags: CGEventFlags = []
    if arguments.flag("command") { flags.insert(.maskCommand) }
    if arguments.flag("shift") { flags.insert(.maskShift) }
    if arguments.flag("option") { flags.insert(.maskAlternate) }
    if arguments.flag("control") { flags.insert(.maskControl) }
    postChord(code, flags: flags, to: pid)

case "click":
    let pid = targetPid()
    let window = prepare(pid)
    let point = screenPoint(in: window, fraction("at"))
    guard windowUnder(point, belongsTo: pid) else {
        fail("another window covers the click point; nothing was posted")
    }
    let pointerBefore = CGEvent(source: nil)?.location
    postClick(at: point, count: arguments.flag("double") ? 2 : 1)
    if let pointerBefore { CGWarpMouseCursorPosition(pointerBefore) }

case "target":
    runTarget(delayMs: arguments.double("delay-ms", 0))

default:
    print(usage)
    exit(command.isEmpty || command == "help" ? 0 : 2)
}
