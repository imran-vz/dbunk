import AppKit
import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let entries = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
print("active=\(NSRunningApplication(processIdentifier: pid)?.isActive ?? false)")
for entry in entries {
    guard entry[kCGWindowLayer as String] as? Int == 0 else { continue }
    print("pid=\(entry[kCGWindowOwnerPID as String] ?? "?") owner=\(entry[kCGWindowOwnerName as String] ?? "?") bounds=\(entry[kCGWindowBounds as String] ?? "?")")
}
