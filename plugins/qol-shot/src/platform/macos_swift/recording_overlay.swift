private struct RecordingOverlayGeometry {
    let hole: NSRect
    let corners: CaptureCorners

    static func resolve(captureRect: CGRect, displayRect: CGRect, bounds: NSRect) -> RecordingOverlayGeometry? {
        let intersection = captureRect.intersection(displayRect)
        guard !intersection.isNull && !intersection.isEmpty else {
            return nil
        }

        let x = intersection.minX - displayRect.minX
        let yFromTop = intersection.minY - displayRect.minY
        let y = bounds.height - yFromTop - intersection.height
        let hole = NSRect(x: x, y: y, width: intersection.width, height: intersection.height)
        let edges = CaptureEdges(
            left: approximatelyEqual(intersection.minX, captureRect.minX),
            right: approximatelyEqual(intersection.maxX, captureRect.maxX),
            top: approximatelyEqual(intersection.minY, captureRect.minY),
            bottom: approximatelyEqual(intersection.maxY, captureRect.maxY)
        )
        return RecordingOverlayGeometry(hole: hole, corners: CaptureCorners(edges: edges))
    }
}

private struct CaptureEdges {
    let left: Bool
    let right: Bool
    let top: Bool
    let bottom: Bool
}

private struct CaptureCorners: OptionSet {
    let rawValue: Int

    static let topLeft = CaptureCorners(rawValue: 1 << 0)
    static let topRight = CaptureCorners(rawValue: 1 << 1)
    static let bottomLeft = CaptureCorners(rawValue: 1 << 2)
    static let bottomRight = CaptureCorners(rawValue: 1 << 3)

    init(rawValue: Int) {
        self.rawValue = rawValue
    }

    init(edges: CaptureEdges) {
        var corners = CaptureCorners()
        if edges.left && edges.top {
            corners.insert(.topLeft)
        }
        if edges.right && edges.top {
            corners.insert(.topRight)
        }
        if edges.left && edges.bottom {
            corners.insert(.bottomLeft)
        }
        if edges.right && edges.bottom {
            corners.insert(.bottomRight)
        }
        self = corners
    }
}

final class RecordingOverlayView: NSView {
    private let captureRect: CGRect
    private let displayRect: CGRect

    init(frame: NSRect, captureRect: CGRect, displayRect: CGRect) {
        self.captureRect = captureRect
        self.displayRect = displayRect
        super.init(frame: NSRect(origin: .zero, size: frame.size))
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override func draw(_ dirtyRect: NSRect) {
        let full = bounds
        guard let geometry = RecordingOverlayGeometry.resolve(
            captureRect: captureRect,
            displayRect: displayRect,
            bounds: bounds
        ) else {
            NSColor(calibratedRed: 0.18, green: 0.50, blue: 0.93, alpha: 0.13).setFill()
            full.fill()
            return
        }

        drawBackdrop(in: full, excluding: geometry.hole)
        drawOuterCorners(geometry.corners, around: geometry.hole)
    }

    private func drawBackdrop(in full: NSRect, excluding hole: NSRect) {
        NSColor(calibratedRed: 0.18, green: 0.50, blue: 0.93, alpha: 0.13).setFill()
        NSRect(x: full.minX, y: hole.maxY, width: full.width, height: full.maxY - hole.maxY).fill()
        NSRect(x: full.minX, y: full.minY, width: full.width, height: hole.minY - full.minY).fill()
        NSRect(x: full.minX, y: hole.minY, width: hole.minX - full.minX, height: hole.height).fill()
        NSRect(x: hole.maxX, y: hole.minY, width: full.maxX - hole.maxX, height: hole.height).fill()
    }

    private func drawOuterCorners(_ corners: CaptureCorners, around rect: NSRect) {
        let length: CGFloat = 28
        let gap: CGFloat = 3
        let lineWidth: CGFloat = 2
        NSColor(calibratedRed: 0.25, green: 0.62, blue: 1.0, alpha: 0.86).setStroke()

        func stroke(_ from: NSPoint, _ to: NSPoint) {
            let path = NSBezierPath()
            path.lineWidth = lineWidth
            path.move(to: from)
            path.line(to: to)
            path.stroke()
        }

        if corners.contains(.topLeft) {
            stroke(NSPoint(x: rect.minX - gap - length, y: rect.maxY + gap), NSPoint(x: rect.minX - gap, y: rect.maxY + gap))
            stroke(NSPoint(x: rect.minX - gap, y: rect.maxY + gap), NSPoint(x: rect.minX - gap, y: rect.maxY + gap + length))
        }
        if corners.contains(.topRight) {
            stroke(NSPoint(x: rect.maxX + gap, y: rect.maxY + gap), NSPoint(x: rect.maxX + gap + length, y: rect.maxY + gap))
            stroke(NSPoint(x: rect.maxX + gap, y: rect.maxY + gap), NSPoint(x: rect.maxX + gap, y: rect.maxY + gap + length))
        }
        if corners.contains(.bottomLeft) {
            stroke(NSPoint(x: rect.minX - gap - length, y: rect.minY - gap), NSPoint(x: rect.minX - gap, y: rect.minY - gap))
            stroke(NSPoint(x: rect.minX - gap, y: rect.minY - gap), NSPoint(x: rect.minX - gap, y: rect.minY - gap - length))
        }
        if corners.contains(.bottomRight) {
            stroke(NSPoint(x: rect.maxX + gap, y: rect.minY - gap), NSPoint(x: rect.maxX + gap + length, y: rect.minY - gap))
            stroke(NSPoint(x: rect.maxX + gap, y: rect.minY - gap), NSPoint(x: rect.maxX + gap, y: rect.minY - gap - length))
        }
    }
}

final class RecordingOverlayWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

let environment = ProcessInfo.processInfo.environment

func readDouble(_ name: String) -> Double? {
    environment[name].flatMap(Double.init)
}

guard
    let x = readDouble("QOL_RECORDING_RECT_X"),
    let y = readDouble("QOL_RECORDING_RECT_Y"),
    let width = readDouble("QOL_RECORDING_RECT_WIDTH"),
    let height = readDouble("QOL_RECORDING_RECT_HEIGHT")
else {
    exit(64)
}

let captureRect = CGRect(x: x, y: y, width: width, height: height)
let maxLifetimeMs = Int(environment["QOL_RECORDING_OVERLAY_MAX_LIFETIME_MS"] ?? "") ?? 0
let targetDisplayRect: CGRect? = {
    guard
        let x = readDouble("QOL_RECORDING_DISPLAY_X"),
        let y = readDouble("QOL_RECORDING_DISPLAY_Y"),
        let width = readDouble("QOL_RECORDING_DISPLAY_WIDTH"),
        let height = readDouble("QOL_RECORDING_DISPLAY_HEIGHT")
    else {
        return nil
    }

    return CGRect(x: x, y: y, width: width, height: height)
}()

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

let terminateSource = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
let interruptSource = DispatchSource.makeSignalSource(signal: SIGINT, queue: .main)
signal(SIGTERM, SIG_IGN)
signal(SIGINT, SIG_IGN)
terminateSource.setEventHandler { NSApp.terminate(nil) }
interruptSource.setEventHandler { NSApp.terminate(nil) }
terminateSource.resume()
interruptSource.resume()

func displayID(for screen: NSScreen) -> CGDirectDisplayID? {
    if let value = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber {
        return CGDirectDisplayID(value.uint32Value)
    }
    return nil
}

func activeDisplayIDs() -> [CGDirectDisplayID] {
    var displays = [CGDirectDisplayID](repeating: 0, count: 32)
    var count: UInt32 = 0
    let result = displays.withUnsafeMutableBufferPointer { buffer in
        CGGetActiveDisplayList(UInt32(buffer.count), buffer.baseAddress, &count)
    }

    guard result == .success && count > 0 else {
        return NSScreen.screens.compactMap { displayID(for: $0) }
    }

    return Array(displays.prefix(Int(count)))
}

func overlayTargets() -> [(displayID: CGDirectDisplayID, screen: NSScreen)] {
    var screensByDisplay: [CGDirectDisplayID: NSScreen] = [:]
    for screen in NSScreen.screens {
        if let displayID = displayID(for: screen), screensByDisplay[displayID] == nil {
            screensByDisplay[displayID] = screen
        }
    }
    let targets: [(displayID: CGDirectDisplayID, screen: NSScreen)] = activeDisplayIDs().compactMap { displayID in
        screensByDisplay[displayID].map { screen in
            (displayID: displayID, screen: screen)
        }
    }

    if let targetDisplayRect {
        return targets.filter { target in
            displayBoundsMatch(CGDisplayBounds(target.displayID), targetDisplayRect)
        }
    }

    guard !targets.isEmpty else {
        return NSScreen.screens.map { screen in
            (displayID: displayID(for: screen) ?? CGMainDisplayID(), screen: screen)
        }
    }

    return targets
}

func approximatelyEqual(_ left: CGFloat, _ right: CGFloat) -> Bool {
    abs(left - right) < 2
}

func displayBoundsMatch(_ left: CGRect, _ right: CGRect) -> Bool {
    approximatelyEqual(left.origin.x, right.origin.x)
        && approximatelyEqual(left.origin.y, right.origin.y)
        && approximatelyEqual(left.width, right.width)
        && approximatelyEqual(left.height, right.height)
}

var windows: [RecordingOverlayWindow] = []
for target in overlayTargets() {
    let screen = target.screen
    let displayRect = CGDisplayBounds(target.displayID)
    let window = RecordingOverlayWindow(
        contentRect: screen.frame,
        styleMask: [.borderless],
        backing: .buffered,
        defer: false,
        screen: screen
    )
    window.level = .screenSaver
    window.canHide = false
    window.backgroundColor = .clear
    window.isOpaque = false
    window.hasShadow = false
    window.ignoresMouseEvents = true
    excludeWindowFromScreenCapture(window)
    window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
    let view = RecordingOverlayView(frame: screen.frame, captureRect: captureRect, displayRect: displayRect)
    view.needsDisplay = true
    window.contentView = view
    window.setFrame(screen.frame, display: true)
    window.orderFrontRegardless()
    window.displayIfNeeded()
    windows.append(window)
}

if maxLifetimeMs > 0 {
    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(maxLifetimeMs)) {
        NSApp.terminate(nil)
    }
}

app.run()
