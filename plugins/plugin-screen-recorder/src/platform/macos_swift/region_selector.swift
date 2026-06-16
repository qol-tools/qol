func armSelectionCursor() {
    NSCursor.crosshair.set()
}

func restoreSelectionCursor() {
    NSCursor.arrow.set()
}

func cancelRegionSelection() -> Never {
    restoreSelectionCursor()
    exit(2)
}

final class SelectionWindow: NSWindow {
    override var canBecomeKey: Bool {
        true
    }

    override var canBecomeMain: Bool {
        true
    }
}

final class SelectionView: NSView {
    let displayBounds: CGRect
    var startPoint: NSPoint?
    var currentPoint: NSPoint?

    init(frame: NSRect, displayBounds: CGRect) {
        self.displayBounds = displayBounds
        super.init(frame: NSRect(origin: .zero, size: frame.size))
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var acceptsFirstResponder: Bool {
        true
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window?.acceptsMouseMovedEvents = true
        window?.invalidateCursorRects(for: self)
        armSelectionCursor()
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        addCursorRect(bounds, cursor: NSCursor.crosshair)
    }

    override func cursorUpdate(with event: NSEvent) {
        armSelectionCursor()
    }

    override func mouseMoved(with event: NSEvent) {
        armSelectionCursor()
    }

    override func draw(_ dirtyRect: NSRect) {
        NSColor.black.withAlphaComponent(0.42).setFill()
        bounds.fill()
        drawGuide()

        guard let rect = selectionRect() else {
            return
        }

        NSColor.systemRed.withAlphaComponent(0.34).setFill()
        rect.fill()
        NSColor.white.setStroke()
        let outerPath = NSBezierPath(rect: rect)
        outerPath.lineWidth = 7
        outerPath.stroke()
        NSColor.systemRed.setStroke()
        let path = NSBezierPath(rect: rect)
        path.lineWidth = 4
        path.stroke()
        drawSelectionLabel(rect)
    }

    override func mouseDown(with event: NSEvent) {
        armSelectionCursor()
        let point = convert(event.locationInWindow, from: nil)
        startPoint = point
        currentPoint = point
        needsDisplay = true
    }

    override func mouseDragged(with event: NSEvent) {
        armSelectionCursor()
        currentPoint = convert(event.locationInWindow, from: nil)
        needsDisplay = true
    }

    override func mouseUp(with event: NSEvent) {
        armSelectionCursor()
        currentPoint = convert(event.locationInWindow, from: nil)
        finishSelection()
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 {
            cancelRegionSelection()
        }
    }

    private func selectionRect() -> NSRect? {
        guard let start = startPoint, let current = currentPoint else {
            return nil
        }

        let x = min(start.x, current.x)
        let y = min(start.y, current.y)
        return NSRect(
            x: x,
            y: y,
            width: abs(start.x - current.x),
            height: abs(start.y - current.y)
        )
    }

    private func drawGuide() {
        let title = startPoint == nil ? "Drag to select recording area" : "Release mouse to start recording"
        let width = min(bounds.width - 48, 520)
        let panel = NSRect(x: bounds.midX - width / 2, y: bounds.maxY - 126, width: width, height: 78)
        OverlayText(title: title, subtitle: "Press Esc to cancel", titleSize: 22, subtitleSize: 14)
            .drawPanel(in: panel)
    }

    private func drawSelectionLabel(_ rect: NSRect) {
        guard rect.width >= 180, rect.height >= 80 else {
            return
        }

        let labelRect = NSRect(x: rect.minX + 12, y: rect.midY - 13, width: rect.width - 24, height: 26)
        OverlayText(title: "Recording area", subtitle: nil, titleSize: 18, subtitleSize: 14)
            .drawLabel(in: labelRect)
    }

    private func finishSelection() {
        guard let rect = selectionRect(), rect.width >= 4, rect.height >= 4 else {
            cancelRegionSelection()
        }

        let x = displayBounds.origin.x + rect.minX
        let y = displayBounds.origin.y + bounds.height - rect.maxY
        let line = "\(Int(x.rounded())),\(Int(y.rounded())),\(Int(rect.width.rounded())),\(Int(rect.height.rounded()))\n"
        FileHandle.standardOutput.write(Data(line.utf8))
        restoreSelectionCursor()
        NSApp.terminate(nil)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

let localEscapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { event in
    guard event.keyCode == 53 else {
        return event
    }

    cancelRegionSelection()
}
let globalEscapeMonitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { event in
    if event.keyCode == 53 {
        cancelRegionSelection()
    }
}
_ = localEscapeMonitor
_ = globalEscapeMonitor

var windows: [NSWindow] = []
for screen in NSScreen.screens {
    let displayID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID ?? CGMainDisplayID()
    let view = SelectionView(frame: screen.frame, displayBounds: CGDisplayBounds(displayID))
    let window = SelectionWindow(
        contentRect: screen.frame,
        styleMask: [.borderless],
        backing: .buffered,
        defer: false,
        screen: screen
    )
    window.level = .screenSaver
    window.backgroundColor = .clear
    window.isOpaque = false
    window.hasShadow = false
    window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
    window.contentView = view
    window.makeKeyAndOrderFront(nil)
    window.makeMain()
    window.makeFirstResponder(view)
    windows.append(window)
}

app.activate(ignoringOtherApps: true)
armSelectionCursor()
app.run()
