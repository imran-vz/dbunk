import Darwin
import Foundation

@_silgen_name("responsibility_get_pid_responsible_for_pid")
private func responsiblePid(_ pid: pid_t) -> pid_t

private func allPids() -> [pid_t] {
    let capacity = Int(proc_listallpids(nil, 0)) + 64
    var pids = [pid_t](repeating: 0, count: capacity)
    let count = Int(proc_listallpids(&pids, Int32(capacity * MemoryLayout<pid_t>.size)))
    return Array(pids.prefix(max(count, 0))).filter { $0 > 0 }
}

private func parent(of pid: pid_t) -> pid_t? {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.size)
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else { return nil }
    return pid_t(info.pbi_ppid)
}

func processName(_ pid: pid_t) -> String {
    var buffer = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
    guard proc_pidpath(pid, &buffer, UInt32(buffer.count)) > 0 else { return "?" }
    return URL(fileURLWithPath: String(cString: buffer)).lastPathComponent
}

private func startTime(of pid: pid_t) -> UInt64? {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.size)
    guard proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size else { return nil }
    return info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec
}

/// The app and every process that exists on its behalf.
///
/// Descendants are found through parent links. WebKit's WebContent, GPU and
/// Networking services are not descendants: launchd starts them, and their
/// responsible PID is whatever is responsible for the app. For an app
/// launched from Finder that is the app itself. For one launched from a
/// shell it is the terminal, which the app shares with its helpers, so a
/// WebKit service also counts when it has the app's responsible PID and
/// started after the app did. Do not run two WebView apps from one terminal
/// while measuring.
func processTree(of root: pid_t) -> [pid_t] {
    let everything = allPids()
    let rootResponsible = responsiblePid(root)
    let rootStarted = startTime(of: root) ?? 0
    var members: Set<pid_t> = [root]
    var grew = true
    while grew {
        grew = false
        for pid in everything where !members.contains(pid) {
            let viaParent = parent(of: pid).map(members.contains) ?? false
            let responsible = responsiblePid(pid)
            let viaResponsibility = members.contains(responsible)
            let sharedTerminal =
                rootResponsible != root && responsible == rootResponsible
                && processName(pid).hasPrefix("com.apple.WebKit.")
                && (startTime(of: pid) ?? 0) >= rootStarted
            if viaParent || viaResponsibility || sharedTerminal {
                members.insert(pid)
                grew = true
            }
        }
    }
    return members.sorted()
}

struct ProcessSample {
    let pid: pid_t
    let footprintBytes: UInt64
    /// User plus system CPU time since the process started, in nanoseconds.
    let cpuNanos: UInt64
}

func sample(_ pid: pid_t) -> ProcessSample? {
    var usage = rusage_info_v4()
    let status = withUnsafeMutablePointer(to: &usage) {
        $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) {
            proc_pid_rusage(pid, RUSAGE_INFO_V4, $0)
        }
    }
    guard status == 0 else { return nil }
    return ProcessSample(
        pid: pid,
        footprintBytes: usage.ri_phys_footprint,
        cpuNanos: Clock.nanos(usage.ri_user_time + usage.ri_system_time))
}
