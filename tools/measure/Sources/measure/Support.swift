import AppKit
import CoreGraphics
import Darwin
import Foundation

// MARK: - Time

/// Mach absolute time is what `CGEvent` timestamps and ScreenCaptureKit's
/// `displayTime` both use, so intervals between them need no clock mapping.
enum Clock {
    private static let timebase: mach_timebase_info_data_t = {
        var info = mach_timebase_info_data_t()
        mach_timebase_info(&info)
        return info
    }()

    static func now() -> UInt64 { mach_absolute_time() }

    static func nanos(_ ticks: UInt64) -> UInt64 {
        ticks * UInt64(timebase.numer) / UInt64(timebase.denom)
    }

    static func millis(from start: UInt64, to end: UInt64) -> Double {
        let ticks = end >= start ? end - start : 0
        return Double(nanos(ticks)) / 1_000_000
    }
}

// MARK: - Arguments

struct Arguments {
    private var values: [String: String] = [:]
    private(set) var rest: [String] = []

    init(_ raw: [String]) {
        var index = 0
        while index < raw.count {
            let item = raw[index]
            if item == "--" {
                rest.append(contentsOf: raw[(index + 1)...])
                break
            }
            if item.hasPrefix("--") {
                let key = String(item.dropFirst(2))
                if index + 1 < raw.count, !raw[index + 1].hasPrefix("--") {
                    values[key] = raw[index + 1]
                    index += 2
                } else {
                    values[key] = "true"
                    index += 1
                }
            } else {
                rest.append(item)
                index += 1
            }
        }
    }

    func string(_ key: String) -> String? { values[key] }
    func int(_ key: String, _ fallback: Int) -> Int { values[key].flatMap(Int.init) ?? fallback }
    func double(_ key: String, _ fallback: Double) -> Double {
        values[key].flatMap(Double.init) ?? fallback
    }
    func flag(_ key: String) -> Bool { values[key] != nil }

    func required(_ key: String) -> String {
        guard let value = values[key] else { fail("missing --\(key)") }
        return value
    }
}

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("measure: \(message)\n".utf8))
    exit(2)
}

func note(_ message: String) {
    FileHandle.standardError.write(Data("\(message)\n".utf8))
}

// MARK: - Statistics

func percentile(_ sorted: [Double], _ p: Double) -> Double {
    guard !sorted.isEmpty else { return .nan }
    let rank = p * Double(sorted.count - 1)
    let low = Int(rank.rounded(.down))
    let high = Int(rank.rounded(.up))
    return sorted[low] + (sorted[high] - sorted[low]) * (rank - Double(low))
}

func summary(_ samples: [Double]) -> [String: Any] {
    let sorted = samples.sorted()
    guard !sorted.isEmpty else { return ["count": 0] }
    return [
        "count": sorted.count,
        "min": sorted.first!,
        "p50": percentile(sorted, 0.50),
        "p95": percentile(sorted, 0.95),
        "p99": percentile(sorted, 0.99),
        "max": sorted.last!,
        "mean": sorted.reduce(0, +) / Double(sorted.count),
    ]
}

// MARK: - Environment record

/// What has to be equal between two runs for them to be comparable.
func environmentRecord() -> [String: Any] {
    var record: [String: Any] = [:]
    var size = 0
    sysctlbyname("hw.model", nil, &size, nil, 0)
    var model = [CChar](repeating: 0, count: size)
    sysctlbyname("hw.model", &model, &size, nil, 0)
    record["model"] = String(cString: model)
    record["os"] = ProcessInfo.processInfo.operatingSystemVersionString
    record["lowPowerMode"] = ProcessInfo.processInfo.isLowPowerModeEnabled
    record["thermalState"] = ProcessInfo.processInfo.thermalState.rawValue
    if let screen = NSScreen.main {
        record["displayPoints"] = [screen.frame.width, screen.frame.height]
        record["displayScale"] = screen.backingScaleFactor
        record["displayMaxHz"] = screen.maximumFramesPerSecond
    }
    let power = Process()
    power.executableURL = URL(fileURLWithPath: "/usr/bin/pmset")
    power.arguments = ["-g", "batt"]
    let pipe = Pipe()
    power.standardOutput = pipe
    if (try? power.run()) != nil {
        power.waitUntilExit()
        let text = String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)
        record["power"] = text?.split(separator: "\n").first.map(String.init) ?? "unknown"
    }
    record["recordedAt"] = ISO8601DateFormatter().string(from: Date())
    return record
}

func emit(_ object: [String: Any], to path: String?) {
    var object = object
    object["environment"] = environmentRecord()
    let data = try! JSONSerialization.data(
        withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
    if let path {
        try! data.write(to: URL(fileURLWithPath: path))
        note("wrote \(path)")
    } else {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }
}

// MARK: - Windows

struct WindowInfo {
    let id: CGWindowID
    let pid: pid_t
    let bounds: CGRect
    let onScreen: Bool
}

/// Normal-layer windows owned by `pid`, largest first.
func windows(of pid: pid_t) -> [WindowInfo] {
    let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]] ?? []
    return list.compactMap { entry -> WindowInfo? in
        guard let owner = entry[kCGWindowOwnerPID as String] as? pid_t, owner == pid,
            let layer = entry[kCGWindowLayer as String] as? Int, layer == 0,
            let id = entry[kCGWindowNumber as String] as? CGWindowID,
            let rect = entry[kCGWindowBounds as String] as? [String: Any],
            let bounds = CGRect(dictionaryRepresentation: rect as CFDictionary),
            bounds.width > 100, bounds.height > 100
        else { return nil }
        let onScreen = entry[kCGWindowIsOnscreen as String] as? Bool ?? false
        return WindowInfo(id: id, pid: pid, bounds: bounds, onScreen: onScreen)
    }
    .sorted { $0.bounds.width * $0.bounds.height > $1.bounds.width * $1.bounds.height }
}

func mainWindow(of pid: pid_t, waitSeconds: Double = 0) -> WindowInfo? {
    let deadline = Date().addingTimeInterval(waitSeconds)
    repeat {
        if let window = windows(of: pid).first(where: { $0.onScreen }) { return window }
        usleep(5_000)
    } while Date() < deadline
    return windows(of: pid).first
}

func activate(_ pid: pid_t) {
    NSRunningApplication(processIdentifier: pid)?.activate(options: [.activateAllWindows])
}
