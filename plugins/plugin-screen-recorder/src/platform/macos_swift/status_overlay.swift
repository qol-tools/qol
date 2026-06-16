final class StatusView: NSView {
    private var title: String
    private var subtitle: String?

    init(frame: NSRect, title: String, subtitle: String?) {
        self.title = title
        self.subtitle = subtitle
        super.init(frame: NSRect(origin: .zero, size: frame.size))
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    func configure(title: String, subtitle: String?) {
        self.title = title
        self.subtitle = subtitle
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        let panelWidth = min(max(bounds.width - 48, 280), 560)
        let panel = NSRect(
            x: bounds.midX - panelWidth / 2,
            y: bounds.maxY - 132,
            width: panelWidth,
            height: 78
        )
        OverlayText(title: title, subtitle: subtitle, titleSize: 22, subtitleSize: 14)
            .drawPanel(in: panel)
    }
}

struct StatusCommand {
    let title: String
    let subtitle: String?
    let durationMs: Int
    let exitAfterHide: Bool
}

let environment = ProcessInfo.processInfo.environment
let title = environment["QOL_STATUS_TITLE"] ?? "Recording ended"
let rawSubtitle = environment["QOL_STATUS_SUBTITLE"] ?? ""
let subtitle = rawSubtitle.isEmpty ? nil : rawSubtitle
let durationMs = max(300, Int(environment["QOL_STATUS_DURATION_MS"] ?? "") ?? 1800)
let exitAfterHide = environment["QOL_STATUS_EXIT_AFTER_HIDE"] == "1"

let app = NSApplication.shared
app.setActivationPolicy(.accessory)

guard let screen = NSScreen.main ?? NSScreen.screens.first else {
    exit(0)
}

let window = NSWindow(
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
window.ignoresMouseEvents = true
window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
let statusView = StatusView(frame: screen.frame, title: title, subtitle: subtitle)
window.contentView = statusView

var hideWorkItem: DispatchWorkItem?
func showStatus(_ command: StatusCommand) {
    hideWorkItem?.cancel()
    statusView.configure(title: command.title, subtitle: command.subtitle)
    window.orderFrontRegardless()

    guard command.exitAfterHide else {
        return
    }

    let workItem = DispatchWorkItem {
        window.orderOut(nil)
        NSApp.terminate(nil)
    }
    hideWorkItem = workItem
    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(command.durationMs), execute: workItem)
}

showStatus(StatusCommand(
    title: title,
    subtitle: subtitle,
    durationMs: durationMs,
    exitAfterHide: exitAfterHide
))

app.run()
