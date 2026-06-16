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
let commandFile = environment["QOL_STATUS_COMMAND_FILE"] ?? ""
let readyFile = environment["QOL_STATUS_READY_FILE"] ?? ""
let serverMode = environment["QOL_STATUS_SERVER"] == "1"

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

    let workItem = DispatchWorkItem {
        window.orderOut(nil)
        if command.exitAfterHide {
            NSApp.terminate(nil)
        }
    }
    hideWorkItem = workItem
    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(command.durationMs), execute: workItem)
}

func readStatusCommand() -> StatusCommand? {
    guard !commandFile.isEmpty else {
        return nil
    }
    guard let raw = try? String(contentsOfFile: commandFile, encoding: .utf8) else {
        return nil
    }
    let lines = raw.split(separator: "\n", omittingEmptySubsequences: false).map(String.init)
    guard lines.count >= 4 else {
        return nil
    }
    return StatusCommand(
        title: lines[0],
        subtitle: lines[1].isEmpty ? nil : lines[1],
        durationMs: max(300, Int(lines[2]) ?? 1800),
        exitAfterHide: lines[3] == "1"
    )
}

if serverMode {
    signal(SIGUSR1, SIG_IGN)
    let source = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
    source.setEventHandler {
        guard let command = readStatusCommand() else {
            return
        }
        showStatus(command)
    }
    source.resume()
    if !readyFile.isEmpty {
        FileManager.default.createFile(atPath: readyFile, contents: Data(), attributes: nil)
    }
} else {
    showStatus(StatusCommand(
        title: title,
        subtitle: subtitle,
        durationMs: durationMs,
        exitAfterHide: true
    ))
}

app.run()
