import CoreMedia
import Foundation
import ScreenCaptureKit

/// Side of the square tiles a frame is compared in, in capture pixels.
let tileSize = 32

/// One frame whose pixels differ from the frame before it.
struct Frame {
    /// Mach absolute time at which the frame was displayed.
    let displayTime: UInt64
    /// Tiles that changed, row-major, `tilesWide` per row.
    let changed: [Bool]
    let tilesWide: Int

    var changedTiles: Int { changed.lazy.filter { $0 }.count }

    /// Changed tiles whose origin lies inside `rect` (capture pixels).
    func changedTiles(in rect: CGRect) -> Int {
        var count = 0
        for (index, isChanged) in changed.enumerated() where isChanged {
            let x = CGFloat((index % tilesWide) * tileSize)
            let y = CGFloat((index / tilesWide) * tileSize)
            if rect.contains(CGPoint(x: x, y: y)) { count += 1 }
        }
        return count
    }
}

/// Keeps the frames in which the captured pixels changed.
///
/// ScreenCaptureKit delivers frames for a window whether or not it repainted,
/// and reports the whole window as dirty each time, so its own status and
/// dirty rectangles cannot tell a response from an idle refresh. Every frame
/// is compared with the previous one here instead.
final class FrameLog: NSObject, SCStreamOutput, SCStreamDelegate {
    private let lock = NSLock()
    private var frames: [Frame] = []
    private var previous = [UInt8]()
    private var previousGeometry = (width: 0, height: 0, bytesPerRow: 0)
    private(set) var delivered = 0
    private(set) var stopped: Error?

    func stream(
        _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard type == .screen,
            let attachments = CMSampleBufferGetSampleAttachmentsArray(
                sampleBuffer, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
            let info = attachments.first,
            let rawStatus = info[.status] as? Int,
            SCFrameStatus(rawValue: rawStatus) == .complete,
            let displayTime = info[.displayTime] as? UInt64,
            let pixels = CMSampleBufferGetImageBuffer(sampleBuffer)
        else { return }
        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixels) else { return }
        let width = CVPixelBufferGetWidth(pixels)
        let height = CVPixelBufferGetHeight(pixels)
        let bytesPerRow = CVPixelBufferGetBytesPerRow(pixels)
        let geometry = (width: width, height: height, bytesPerRow: bytesPerRow)
        let current = base.assumingMemoryBound(to: UInt8.self)
        let byteCount = bytesPerRow * height

        var changed: [Bool]?
        let tilesWide = (width + tileSize - 1) / tileSize
        let tilesHigh = (height + tileSize - 1) / tileSize
        if previous.count == byteCount, previousGeometry == geometry {
            previous.withUnsafeBufferPointer { old in
                guard let oldBase = old.baseAddress else { return }
                for y in 0..<height {
                    let rowOffset = y * bytesPerRow
                    // Whole rows are compared first: most rows of most frames
                    // are identical.
                    if memcmp(current + rowOffset, oldBase + rowOffset, width * 4) == 0 { continue }
                    if changed == nil {
                        changed = [Bool](repeating: false, count: tilesWide * tilesHigh)
                    }
                    let tileRow = (y / tileSize) * tilesWide
                    for tile in 0..<tilesWide where !changed![tileRow + tile] {
                        let x = tile * tileSize
                        let bytes = min(tileSize, width - x) * 4
                        if memcmp(current + rowOffset + x * 4, oldBase + rowOffset + x * 4, bytes)
                            != 0
                        {
                            changed![tileRow + tile] = true
                        }
                    }
                }
            }
        }
        if previous.count != byteCount { previous = [UInt8](repeating: 0, count: byteCount) }
        previous.withUnsafeMutableBufferPointer { old in
            _ = memcpy(old.baseAddress!, current, byteCount)
        }
        previousGeometry = geometry

        lock.lock()
        delivered += 1
        if let changed {
            frames.append(Frame(displayTime: displayTime, changed: changed, tilesWide: tilesWide))
        }
        lock.unlock()
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        lock.lock()
        stopped = error
        lock.unlock()
    }

    func snapshot() -> [Frame] {
        lock.lock()
        defer { lock.unlock() }
        return frames
    }

    /// The first changed frame displayed after `time`, waiting up to
    /// `timeoutMs`.
    func firstFrame(after time: UInt64, timeoutMs: Double) -> Frame? {
        let deadline = Date().addingTimeInterval(timeoutMs / 1000)
        var scanned = 0
        while true {
            lock.lock()
            let current = frames
            lock.unlock()
            // Frames arrive in display order, so only the new tail is scanned.
            for frame in current[scanned...] where frame.displayTime > time {
                return frame
            }
            scanned = current.count
            if Date() >= deadline { return nil }
            usleep(500)
        }
    }
}

enum CaptureTarget {
    case window(CGWindowID)
    case display
}

struct Capture {
    let stream: SCStream
    let log: FrameLog
    /// Capture pixels per point.
    let scale: CGFloat
    let refreshHz: Int

    func stop() {
        let done = DispatchSemaphore(value: 0)
        stream.stopCapture { _ in done.signal() }
        _ = done.wait(timeout: .now() + 2)
    }
}

/// Starts a capture at the display's refresh rate. A window target follows
/// that window wherever it is; a display target sees everything, which the
/// startup measurement needs because the window does not exist yet.
func startCapture(_ target: CaptureTarget) -> Capture {
    let ready = DispatchSemaphore(value: 0)
    var shareable: SCShareableContent?
    var failure: Error?
    SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) {
        content, error in
        shareable = content
        failure = error
        ready.signal()
    }
    ready.wait()
    guard let content = shareable else {
        fail("screen capture unavailable: \(failure?.localizedDescription ?? "no content")")
    }
    guard let display = content.displays.first else { fail("no display") }
    let refreshHz = NSScreen.main?.maximumFramesPerSecond ?? 60
    let scale = NSScreen.main?.backingScaleFactor ?? 2

    let filter: SCContentFilter
    let size: CGSize
    switch target {
    case .window(let id):
        guard let window = content.windows.first(where: { $0.windowID == id }) else {
            fail("window \(id) is not shareable")
        }
        filter = SCContentFilter(desktopIndependentWindow: window)
        size = window.frame.size
    case .display:
        filter = SCContentFilter(display: display, excludingWindows: [])
        size = CGSize(width: display.width, height: display.height)
    }

    // A window is captured at native resolution. The whole display is captured
    // at one pixel per point, which keeps the per-frame comparison cheap.
    let captureScale: CGFloat
    switch target {
    case .window: captureScale = scale
    case .display: captureScale = 1
    }
    let configuration = SCStreamConfiguration()
    configuration.width = Int(size.width * captureScale)
    configuration.height = Int(size.height * captureScale)
    configuration.minimumFrameInterval = CMTime(value: 1, timescale: CMTimeScale(refreshHz))
    configuration.queueDepth = 8
    configuration.showsCursor = false
    configuration.pixelFormat = kCVPixelFormatType_32BGRA

    let log = FrameLog()
    let stream = SCStream(filter: filter, configuration: configuration, delegate: log)
    do {
        try stream.addStreamOutput(
            log, type: .screen,
            sampleHandlerQueue: DispatchQueue(label: "measure.frames", qos: .userInteractive))
    } catch {
        fail("cannot attach capture output: \(error.localizedDescription)")
    }
    let started = DispatchSemaphore(value: 0)
    var startError: Error?
    stream.startCapture { error in
        startError = error
        started.signal()
    }
    started.wait()
    if let startError { fail("capture did not start: \(startError.localizedDescription)") }
    return Capture(stream: stream, log: log, scale: captureScale, refreshHz: refreshHz)
}
