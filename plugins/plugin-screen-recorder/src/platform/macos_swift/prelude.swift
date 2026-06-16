import AppKit
import CoreGraphics
import Darwin
import Dispatch
import Foundation

struct OverlayText {
    let title: String
    let subtitle: String?
    let titleSize: CGFloat
    let subtitleSize: CGFloat

    func drawPanel(in panel: NSRect) {
        let panelPath = NSBezierPath(roundedRect: panel, xRadius: 14, yRadius: 14)
        NSColor.black.withAlphaComponent(0.78).setFill()
        panelPath.fill()
        NSColor.white.withAlphaComponent(0.86).setStroke()
        panelPath.lineWidth = 1.5
        panelPath.stroke()

        let contentX = panel.minX + 18
        let contentWidth = panel.width - 36
        let titleHeight: CGFloat = 28

        guard let subtitle else {
            drawLine(
                title,
                in: NSRect(x: contentX, y: panel.midY - titleHeight / 2, width: contentWidth, height: titleHeight),
                size: titleSize,
                weight: .semibold,
                alpha: 1.0
            )
            return
        }

        let subtitleHeight: CGFloat = 20
        let gap: CGFloat = 6
        let contentHeight = titleHeight + gap + subtitleHeight
        let subtitleRect = NSRect(
            x: contentX,
            y: panel.midY - contentHeight / 2,
            width: contentWidth,
            height: subtitleHeight
        )
        drawLine(
            title,
            in: NSRect(x: contentX, y: subtitleRect.maxY + gap, width: contentWidth, height: titleHeight),
            size: titleSize,
            weight: .semibold,
            alpha: 1.0
        )

        drawLine(
            subtitle,
            in: subtitleRect,
            size: subtitleSize,
            weight: .regular,
            alpha: 0.78
        )
    }

    func drawLabel(in rect: NSRect) {
        drawLine(title, in: rect, size: titleSize, weight: .semibold, alpha: 0.96)
    }

    private func drawLine(_ text: String, in rect: NSRect, size: CGFloat, weight: NSFont.Weight, alpha: CGFloat) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: size, weight: weight),
            .foregroundColor: NSColor.white.withAlphaComponent(alpha),
            .paragraphStyle: paragraph
        ]
        text.draw(in: rect, withAttributes: attributes)
    }
}
