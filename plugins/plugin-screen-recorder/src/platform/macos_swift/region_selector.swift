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

final class SelectionWindow: NSPanel {
    override var canBecomeKey: Bool {
        true
    }

    override var canBecomeMain: Bool {
        true
    }
}

struct ScreenGeometry {
    let screen: NSScreen
    let screenFrame: NSRect
    let displayBounds: CGRect

    func containsMouse(_ point: NSPoint) -> Bool {
        screenFrame.contains(point)
    }

    func captureRect(for intersection: NSRect) -> CGRect {
        let localMinX = intersection.minX - screenFrame.minX
        let localMaxY = intersection.maxY - screenFrame.minY
        return CGRect(
            x: displayBounds.origin.x + localMinX,
            y: displayBounds.origin.y + screenFrame.height - localMaxY,
            width: intersection.width,
            height: intersection.height
        )
    }
}

final class SelectionCoordinator {
    private let screens: [ScreenGeometry]
    private var startPoint: NSPoint?
    private var currentPoint: NSPoint?
    private var views: [SelectionView] = []

    init(screens: [ScreenGeometry]) {
        self.screens = screens
    }

    var isSelecting: Bool {
        startPoint != nil
    }

    func attach(_ view: SelectionView) {
        views.append(view)
    }

    func begin(at point: NSPoint) {
        startPoint = point
        currentPoint = point
        invalidateViews()
    }

    func update(to point: NSPoint) {
        guard startPoint != nil else {
            return
        }
        currentPoint = point
        invalidateViews()
    }

    func selectionRect(in screenFrame: NSRect) -> NSRect? {
        guard let selection = selectionRectInScreenCoordinates() else {
            return nil
        }

        let intersection = selection.intersection(screenFrame)
        if intersection.isNull || intersection.isEmpty {
            return nil
        }

        return NSRect(
            x: intersection.minX - screenFrame.minX,
            y: intersection.minY - screenFrame.minY,
            width: intersection.width,
            height: intersection.height
        )
    }

    func finishSelection() {
        guard let captureRect = captureRect(), captureRect.width >= 4, captureRect.height >= 4 else {
            cancelRegionSelection()
        }

        let line = "\(Int(captureRect.origin.x.rounded())),\(Int(captureRect.origin.y.rounded())),\(Int(captureRect.width.rounded())),\(Int(captureRect.height.rounded()))\n"
        FileHandle.standardOutput.write(Data(line.utf8))
        restoreSelectionCursor()
        NSApp.terminate(nil)
    }

    private func selectionRectInScreenCoordinates() -> NSRect? {
        guard let start = startPoint, let current = currentPoint else {
            return nil
        }

        return NSRect(
            x: min(start.x, current.x),
            y: min(start.y, current.y),
            width: abs(start.x - current.x),
            height: abs(start.y - current.y)
        )
    }

    private func captureRect() -> CGRect? {
        guard let selection = selectionRectInScreenCoordinates(), !selection.isNull, !selection.isEmpty else {
            return nil
        }

        var result: CGRect?
        for screen in screens {
            let intersection = selection.intersection(screen.screenFrame)
            if intersection.isNull || intersection.isEmpty {
                continue
            }

            let capture = screen.captureRect(for: intersection)
            result = result.map { $0.union(capture) } ?? capture
        }

        return result
    }

    private func invalidateViews() {
        for view in views {
            view.needsDisplay = true
        }
    }
}

final class SelectionView: NSView {
    let screenGeometry: ScreenGeometry
    let coordinator: SelectionCoordinator
    let showsGuide: Bool

    init(frame: NSRect, screenGeometry: ScreenGeometry, coordinator: SelectionCoordinator, showsGuide: Bool) {
        self.screenGeometry = screenGeometry
        self.coordinator = coordinator
        self.showsGuide = showsGuide
        super.init(frame: NSRect(origin: .zero, size: frame.size))
        coordinator.attach(self)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    override var acceptsFirstResponder: Bool {
        true
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool {
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
        if showsGuide {
            drawGuide()
        }

        guard let rect = coordinator.selectionRect(in: screenGeometry.screenFrame) else {
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
        coordinator.begin(at: screenPoint(for: event))
    }

    override func mouseDragged(with event: NSEvent) {
        armSelectionCursor()
        coordinator.update(to: screenPoint(for: event))
    }

    override func mouseUp(with event: NSEvent) {
        armSelectionCursor()
        coordinator.update(to: screenPoint(for: event))
        coordinator.finishSelection()
    }

    override func keyDown(with event: NSEvent) {
        if event.keyCode == 53 {
            cancelRegionSelection()
        }
    }

    private func screenPoint(for event: NSEvent) -> NSPoint {
        let point = convert(event.locationInWindow, from: nil)
        return NSPoint(
            x: screenGeometry.screenFrame.minX + point.x,
            y: screenGeometry.screenFrame.minY + point.y
        )
    }

    private func drawGuide() {
        let title = coordinator.isSelecting ? "Release mouse to capture" : "Drag to select capture area"
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
        OverlayText(title: "Capture area", subtitle: nil, titleSize: 18, subtitleSize: 14)
            .drawLabel(in: labelRect)
    }
}

func displayBounds(for screen: NSScreen) -> CGRect {
    let displayID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID ?? CGMainDisplayID()
    return CGDisplayBounds(displayID)
}

func screenGeometries() -> [ScreenGeometry] {
    NSScreen.screens.map { screen in
        ScreenGeometry(screen: screen, screenFrame: screen.frame, displayBounds: displayBounds(for: screen))
    }
}

func activeMonitorBoundsFromEnvironment() -> CGRect? {
    let environment = ProcessInfo.processInfo.environment
    guard
        let rawX = environment["QOL_ACTIVE_MONITOR_X"],
        let rawY = environment["QOL_ACTIVE_MONITOR_Y"],
        let rawWidth = environment["QOL_ACTIVE_MONITOR_WIDTH"],
        let rawHeight = environment["QOL_ACTIVE_MONITOR_HEIGHT"],
        let x = Double(rawX),
        let y = Double(rawY),
        let width = Double(rawWidth),
        let height = Double(rawHeight)
    else {
        return nil
    }

    return CGRect(x: x, y: y, width: width, height: height)
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

func activeScreenIndex(in screens: [ScreenGeometry]) -> Int {
    if let active = activeMonitorBoundsFromEnvironment(),
       let index = screens.firstIndex(where: { displayBoundsMatch($0.displayBounds, active) }) {
        return index
    }

    let mouse = NSEvent.mouseLocation
    if let index = screens.firstIndex(where: { $0.containsMouse(mouse) }) {
        return index
    }

    if let main = NSScreen.main,
       let index = screens.firstIndex(where: { $0.screen == main }) {
        return index
    }

    return 0
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

let screens = screenGeometries()
if screens.isEmpty {
    exit(1)
}

app.activate(ignoringOtherApps: true)

let activeIndex = activeScreenIndex(in: screens)
let coordinator = SelectionCoordinator(screens: screens)

var windows: [NSWindow] = []
for (index, screen) in screens.enumerated() {
    let view = SelectionView(
        frame: screen.screenFrame,
        screenGeometry: screen,
        coordinator: coordinator,
        showsGuide: index == activeIndex
    )
    let window = SelectionWindow(
        contentRect: screen.screenFrame,
        styleMask: [.borderless, .nonactivatingPanel],
        backing: .buffered,
        defer: false,
        screen: screen.screen
    )
    window.level = .screenSaver
    window.isFloatingPanel = true
    window.hidesOnDeactivate = false
    window.becomesKeyOnlyIfNeeded = false
    window.canHide = false
    window.backgroundColor = .clear
    window.isOpaque = false
    window.hasShadow = false
    window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
    window.contentView = view
    window.setFrame(screen.screenFrame, display: true)
    window.orderFrontRegardless()
    if index == activeIndex {
        window.makeKeyAndOrderFront(nil)
        window.makeMain()
        window.makeFirstResponder(view)
    }
    windows.append(window)
}

armSelectionCursor()
app.run()
