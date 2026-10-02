// swift-tools-version:5.9
import PackageDescription

// External measurement harness for Plan 024 (GPUI migration stage 01).
// One tool for both desktop hosts: it posts synthetic input to a process and
// reads frame timing from the window server, so neither host's own timers are
// involved.
let package = Package(
    name: "measure",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "measure",
            path: "Sources/measure"
        )
    ]
)
