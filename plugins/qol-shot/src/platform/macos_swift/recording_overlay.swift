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
        guard let hole = localCaptureRect() else {
            NSColor(calibratedRed: 0.18, green: 0.50, blue: 0.93, alpha: 0.13).setFill()
            full.fill()
            return
        }

        NSColor(calibratedRed: 0.18, green: 0.50, blue: 0.93, alpha: 0.13).setFill()
        NSRect(x: full.minX, y: hole.maxY, width: full.width, height: full.maxY - hole.maxY).fill()
        NSRect(x: full.minX, y: full.minY, width: full.width, height: hole.minY - full.minY).fill()
        NSRect(x: full.minX, y: hole.minY, width: hole.minX - full.minX, height: hole.height).fill()
        NSRect(x: hole.maxX, y: hole.minY, width: full.maxX - hole.maxX, height: hole.height).fill()

        drawOuterCorners(around: hole)
    }

    private func localCaptureRect() -> NSRect? {
        let intersection = captureRect.intersection(displayRect)
        guard !intersection.isNull && !intersection.isEmpty else {
            return nil
        }

        let x = intersection.minX - displayRect.minX
        let yFromTop = intersection.minY - displayRect.minY
        let y = bounds.height - yFromTop - intersection.height
        return NSRect(x: x, y: y, width: intersection.width, height: intersection.height)
    }

    private func drawOuterCorners(around rect: NSRect) {
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

        stroke(NSPoint(x: rect.minX - gap - length, y: rect.maxY + gap), NSPoint(x: rect.minX - gap, y: rect.maxY + gap))
        stroke(NSPoint(x: rect.minX - gap, y: rect.maxY + gap), NSPoint(x: rect.minX - gap, y: rect.maxY + gap + length))
        stroke(NSPoint(x: rect.maxX + gap, y: rect.maxY + gap), NSPoint(x: rect.maxX + gap + length, y: rect.maxY + gap))
        stroke(NSPoint(x: rect.maxX + gap, y: rect.maxY + gap), NSPoint(x: rect.maxX + gap, y: rect.maxY + gap + length))
        stroke(NSPoint(x: rect.minX - gap - length, y: rect.minY - gap), NSPoint(x: rect.minX - gap, y: rect.minY - gap))
        stroke(NSPoint(x: rect.minX - gap, y: rect.minY - gap), NSPoint(x: rect.minX - gap, y: rect.minY - gap - length))
        stroke(NSPoint(x: rect.maxX + gap, y: rect.minY - gap), NSPoint(x: rect.maxX + gap + length, y: rect.minY - gap))
        stroke(NSPoint(x: rect.maxX + gap, y: rect.minY - gap), NSPoint(x: rect.maxX + gap, y: rect.minY - gap - length))
    }
}

final class RecordingOverlayWindow: NSPanel {
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

func displayBounds(for screen: NSScreen) -> CGRect {
    let displayID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID ?? CGMainDisplayID()
    return CGDisplayBounds(displayID)
}

var windows: [RecordingOverlayWindow] = []
for screen in NSScreen.screens {
    let displayRect = displayBounds(for: screen)
    let window = RecordingOverlayWindow(
        contentRect: screen.frame,
        styleMask: [.borderless, .nonactivatingPanel],
        backing: .buffered,
        defer: false,
        screen: screen
    )
    window.level = .screenSaver
    window.isFloatingPanel = true
    window.hidesOnDeactivate = false
    window.canHide = false
    window.backgroundColor = .clear
    window.isOpaque = false
    window.hasShadow = false
    window.ignoresMouseEvents = true
    window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
    window.contentView = RecordingOverlayView(frame: screen.frame, captureRect: captureRect, displayRect: displayRect)
    window.orderFrontRegardless()
    windows.append(window)
}

if maxLifetimeMs > 0 {
    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(maxLifetimeMs)) {
        NSApp.terminate(nil)
    }
}

app.run()
