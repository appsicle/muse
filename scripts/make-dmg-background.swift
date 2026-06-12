// Renders the DMG window background as a scrapbook spread: grained cream
// paper, washi-tape strips, a fat hand-drawn crimson drag arrow, a
// typewriter caption, and a little barcode ephemera in the corner.
//
// Usage: swift make-dmg-background.swift <out.png>   (600x360 @2x = 1200x720)
// Icon positions in the DMG window: Muse.app (150,185), Applications (450,185).

import AppKit

guard CommandLine.arguments.count == 2 else {
    fputs("usage: swift make-dmg-background.swift <out.png>\n", stderr)
    exit(1)
}

let size = NSSize(width: 600, height: 360)
let scale: CGFloat = 2.0

// Design tokens.
let paper = NSColor(srgbRed: 0.980, green: 0.973, blue: 0.961, alpha: 1.0)    // #FAF8F5
let ink = NSColor(srgbRed: 0.149, green: 0.133, blue: 0.110, alpha: 1.0)      // #26221C
let crimson = NSColor(srgbRed: 0.843, green: 0.149, blue: 0.239, alpha: 1.0)  // #D7263D
let cobalt = NSColor(srgbRed: 0.133, green: 0.267, blue: 0.800, alpha: 1.0)   // #2244CC
let warmGray = NSColor(srgbRed: 0.659, green: 0.635, blue: 0.588, alpha: 1.0) // #A8A296

// A tiny LCG so the grain and barcode are reproducible run to run.
struct LCG {
    var state: UInt64
    mutating func next() -> UInt64 {
        state = state &* 6364136223846793005 &+ 1442695040888963407
        return state >> 33
    }
    mutating func cgFloat() -> CGFloat { CGFloat(next() % 100_000) / 100_000 }
}

let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil,
    pixelsWide: Int(size.width * scale), pixelsHigh: Int(size.height * scale),
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
)!
rep.size = size // the DMG window depends on 600x360 points
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)

// ── Paper base.
paper.setFill()
NSRect(origin: .zero, size: size).fill()

// ── Subtle paper grain: a few thousand 1px specks at 2-3% alpha.
var rng = LCG(state: 0x5EED_1DEA)
for _ in 0..<3600 {
    let x = rng.cgFloat() * size.width
    let y = rng.cgFloat() * size.height
    let a = 0.02 + rng.cgFloat() * 0.01
    ink.withAlphaComponent(a).setFill()
    NSRect(x: x, y: y, width: 0.5, height: 0.5).fill()
}

// ── Washi-tape strips: rotated translucent rectangles, top corners.
func washi(center: NSPoint, angle: CGFloat, color: NSColor) {
    NSGraphicsContext.current?.saveGraphicsState()
    let t = NSAffineTransform()
    t.translateX(by: center.x, yBy: center.y)
    t.rotate(byDegrees: angle)
    t.concat()
    color.setFill()
    NSRect(x: -55, y: -13, width: 110, height: 26).fill()
    // Faint edge lines so the tape reads as tape, not a tint block.
    color.withAlphaComponent(0.3).setFill()
    NSRect(x: -55, y: 11.5, width: 110, height: 1.5).fill()
    NSRect(x: -55, y: -13, width: 110, height: 1.5).fill()
    NSGraphicsContext.current?.restoreGraphicsState()
}
washi(center: NSPoint(x: 88, y: 330), angle: -4, color: crimson.withAlphaComponent(0.18))
washi(center: NSPoint(x: 512, y: 326), angle: 4, color: cobalt.withAlphaComponent(0.15))

// ── The drag arrow: a fat wavy marker stroke between the two icon wells.
// Muse.app sits at (150,185-from-top) = (150,175) here; Applications at (450,175).
let arrowY: CGFloat = 175
let stroke = NSBezierPath()
stroke.move(to: NSPoint(x: 230, y: arrowY + 1))
stroke.curve(
    to: NSPoint(x: 365, y: arrowY - 0.5),
    controlPoint1: NSPoint(x: 272, y: arrowY + 6),
    controlPoint2: NSPoint(x: 322, y: arrowY - 6)
)
stroke.lineWidth = 7
stroke.lineCapStyle = .round
crimson.setStroke()
stroke.stroke()

// Chunky arrowhead, slightly asymmetric so it feels drawn.
let head = NSBezierPath()
head.move(to: NSPoint(x: 389, y: arrowY - 0.5))
head.line(to: NSPoint(x: 362, y: arrowY + 13))
head.line(to: NSPoint(x: 367, y: arrowY - 1))
head.line(to: NSPoint(x: 361, y: arrowY - 13.5))
head.close()
crimson.setFill()
head.fill()

// ── Caption under the icons, typewriter style.
let captionFont = NSFont(name: "Courier", size: 14) ?? NSFont.systemFont(ofSize: 14)
let caption = NSAttributedString(string: "drag muse into applications", attributes: [
    .font: captionFont,
    .foregroundColor: warmGray,
    .kern: 2.4,
])
let captionBounds = caption.boundingRect(with: size)
caption.draw(at: NSPoint(x: (size.width - captionBounds.width) / 2, y: 84))

// ── Bottom-right ephemera: a tiny fake barcode with a typewriter label.
let barcodeOrigin = NSPoint(x: 506, y: 38)
var barRng = LCG(state: 0xBA2C_0DE5)
var bx = barcodeOrigin.x
ink.withAlphaComponent(0.9).setFill()
while bx < barcodeOrigin.x + 70 {
    let barWidth = 1 + barRng.cgFloat() * 2 // 1-3pt bars
    let gap = 1 + barRng.cgFloat() * 2.2
    NSRect(x: bx, y: barcodeOrigin.y, width: barWidth, height: 22).fill()
    bx += barWidth + gap
}
let labelFont = NSFont(name: "Courier", size: 9) ?? NSFont.systemFont(ofSize: 9)
let label = NSAttributedString(string: "MUSE \u{00B7} \u{2116} 01", attributes: [
    .font: labelFont,
    .foregroundColor: ink.withAlphaComponent(0.75),
    .kern: 1.0,
])
label.draw(at: NSPoint(x: barcodeOrigin.x, y: barcodeOrigin.y - 14))

NSGraphicsContext.restoreGraphicsState()
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
print("background written")
