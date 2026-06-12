// Renders the Muse app icon: a cream postage stamp with perforated edges,
// loud grotesque "MUSE" wordmark, typewriter subline, crimson denomination,
// and a halftone-dotted writing figure — analog print, not quiet minimal.
//
// Usage: swift scripts/make-icon.swift <out-dir>
// Writes <out-dir>/Muse.iconset/icon_*.png; run iconutil afterwards.

import AppKit

let args = CommandLine.arguments
guard args.count == 2 else {
    fputs("usage: swift make-icon.swift <out-dir>\n", stderr)
    exit(1)
}
let iconset = URL(fileURLWithPath: args[1]).appendingPathComponent("Muse.iconset")
try? FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

// Design tokens.
let paper = NSColor(srgbRed: 0.980, green: 0.973, blue: 0.961, alpha: 1.0)   // #FAF8F5
let ink = NSColor(srgbRed: 0.149, green: 0.133, blue: 0.110, alpha: 1.0)     // #26221C
let crimson = NSColor(srgbRed: 0.843, green: 0.149, blue: 0.239, alpha: 1.0) // #D7263D
let warmGray = NSColor(srgbRed: 0.659, green: 0.635, blue: 0.588, alpha: 1.0) // #A8A296

func grotesque(_ size: CGFloat) -> NSFont {
    NSFont(name: "HelveticaNeue-CondensedBlack", size: size)
        ?? NSFont(name: "Helvetica-Bold", size: size)
        ?? NSFont.boldSystemFont(ofSize: size)
}

// Rasterize an SF Symbol silhouette into a bitmap so we can sample its alpha.
func symbolBitmap(name: String, pxWide: Int, pxHigh: Int) -> NSBitmapImageRep? {
    guard let base = NSImage(systemSymbolName: name, accessibilityDescription: nil) else { return nil }
    let config = NSImage.SymbolConfiguration(pointSize: 512, weight: .bold)
    guard let symbol = base.withSymbolConfiguration(config) else { return nil }
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: pxWide, pixelsHigh: pxHigh, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { return nil }
    rep.size = NSSize(width: pxWide, height: pxHigh)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    // Aspect-fit the symbol into the bitmap.
    let sym = symbol.size
    let fit = min(CGFloat(pxWide) / sym.width, CGFloat(pxHigh) / sym.height)
    let w = sym.width * fit, h = sym.height * fit
    symbol.draw(
        in: NSRect(x: (CGFloat(pxWide) - w) / 2, y: (CGFloat(pxHigh) - h) / 2, width: w, height: h),
        from: .zero, operation: .sourceOver, fraction: 1.0
    )
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

// Draws the perforated stamp (no shadow) so its alpha mask is exact;
// the caller re-draws it with an NSShadow that hugs the perforations.
func drawStamp(canvas: Int) -> NSImage {
    let s = CGFloat(canvas)
    let image = NSImage(size: NSSize(width: s, height: s))
    image.lockFocus()

    // ── The stamp body: cream rectangle inset ~9%.
    let inset = s * 0.09
    let stamp = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    paper.setFill()
    NSBezierPath(rect: stamp).fill()

    // ── Perforations: punch semicircular notches along all four edges.
    // Circles centered ON the edge line, erased with destination-out.
    NSGraphicsContext.current?.saveGraphicsState()
    NSGraphicsContext.current?.compositingOperation = .destinationOut
    let perfR = s * 0.0215            // ~22 px at 1024
    let span = stamp.width
    let holes = Int(round(span / (s * 0.0605)))  // ~62 px pitch at 1024
    let pitch = span / CGFloat(holes)
    NSColor.black.setFill()
    for i in 0...holes {
        let t = stamp.minX + CGFloat(i) * pitch
        for center in [
            NSPoint(x: t, y: stamp.minY), NSPoint(x: t, y: stamp.maxY),
            NSPoint(x: stamp.minX, y: t), NSPoint(x: stamp.maxX, y: t),
        ] {
            NSBezierPath(ovalIn: NSRect(
                x: center.x - perfR, y: center.y - perfR,
                width: perfR * 2, height: perfR * 2
            )).fill()
        }
    }
    NSGraphicsContext.current?.restoreGraphicsState()

    // ── Thin warm-gray border line inside the perforation.
    let borderInset = perfR + s * 0.016
    let border = NSBezierPath(rect: stamp.insetBy(dx: borderInset, dy: borderInset))
    warmGray.withAlphaComponent(0.85).setStroke()
    border.lineWidth = max(1, s * 0.004)
    border.stroke()

    // ── Type block, top-left: heavy grotesque "MUSE", typewriter subline.
    let margin = stamp.minX + borderInset + s * 0.035
    let muse = NSAttributedString(string: "MUSE", attributes: [
        .font: grotesque(s * 0.1455),
        .foregroundColor: ink,
        .kern: s * 0.002,
    ])
    let museBounds = muse.boundingRect(with: NSSize(width: s, height: s))
    let museTop = stamp.maxY - borderInset - s * 0.030
    let museY = museTop - museBounds.height - museBounds.origin.y
    muse.draw(at: NSPoint(x: margin - museBounds.origin.x, y: museY))

    let subline = NSAttributedString(string: "FIELD NOTES", attributes: [
        .font: NSFont(name: "Courier-Bold", size: s * 0.039) ?? NSFont.boldSystemFont(ofSize: s * 0.039),
        .foregroundColor: warmGray,
        .kern: s * 0.0125,
    ])
    subline.draw(at: NSPoint(x: margin + s * 0.004, y: museY - s * 0.052))

    // ── Denomination, top-right: loud crimson "01".
    let denom = NSAttributedString(string: "01", attributes: [
        .font: grotesque(s * 0.105),
        .foregroundColor: crimson,
    ])
    let denomBounds = denom.boundingRect(with: NSSize(width: s, height: s))
    denom.draw(at: NSPoint(
        x: stamp.maxX - borderInset - s * 0.035 - denomBounds.width - denomBounds.origin.x,
        y: museTop - denomBounds.height - denomBounds.origin.y
    ))

    // ── Halftone figure: crimson dots sampled from an SF Symbol silhouette,
    // filling the lower two-thirds of the stamp like the print poster.
    let cell = s * 0.0254             // ~26 px grid at 1024
    let maxDot = s * 0.01075          // ~11 px max radius at 1024
    let region = NSRect(
        x: stamp.minX + borderInset + s * 0.026,
        y: stamp.minY + borderInset + s * 0.022,
        width: stamp.width - 2 * (borderInset + s * 0.026),
        height: s * 0.50
    )
    let cols = Int(region.width / cell)
    let rows = Int(region.height / cell)
    let ss = 4 // supersampling factor: average alpha over an ss x ss block per cell
    if let sym = symbolBitmap(name: "square.and.pencil", pxWide: cols * ss, pxHigh: rows * ss) {
        crimson.setFill()
        for row in 0..<rows {
            for col in 0..<cols {
                // colorAt is top-left origin; our canvas is bottom-left.
                var sum: CGFloat = 0
                for dy in 0..<ss {
                    for dx in 0..<ss {
                        sum += sym.colorAt(x: col * ss + dx, y: (rows - 1 - row) * ss + dy)?
                            .alphaComponent ?? 0
                    }
                }
                let alpha = sum / CGFloat(ss * ss)
                guard alpha > 0.08 else { continue }
                let r = maxDot * sqrt(alpha) // area-true halftone ramp
                let cx = region.minX + (CGFloat(col) + 0.5) * cell
                let cy = region.minY + (CGFloat(row) + 0.5) * cell
                NSBezierPath(ovalIn: NSRect(x: cx - r, y: cy - r, width: r * 2, height: r * 2)).fill()
            }
        }
    }

    image.unlockFocus()
    return image
}

// Composite the perforated stamp over a transparent canvas with a soft
// drop shadow that follows the notched outline.
func draw(canvas: Int) -> NSImage {
    let s = CGFloat(canvas)
    let stamp = drawStamp(canvas: canvas)
    let image = NSImage(size: NSSize(width: s, height: s))
    image.lockFocus()
    NSGraphicsContext.current?.saveGraphicsState()
    let shadow = NSShadow()
    shadow.shadowColor = NSColor.black.withAlphaComponent(0.25)
    shadow.shadowBlurRadius = s * 0.022
    shadow.shadowOffset = NSSize(width: 0, height: -s * 0.011)
    shadow.set()
    stamp.draw(in: NSRect(x: 0, y: 0, width: s, height: s))
    NSGraphicsContext.current?.restoreGraphicsState()
    image.unlockFocus()
    return image
}

func write(_ image: NSImage, px: Int, name: String) {
    guard let tiff = image.tiffRepresentation,
          let rep = NSBitmapImageRep(data: tiff) else { exit(3) }
    rep.size = NSSize(width: px, height: px)
    guard let resized = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ) else { exit(3) }
    resized.size = NSSize(width: px, height: px)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: resized)
    NSGraphicsContext.current?.imageInterpolation = .high
    image.draw(in: NSRect(x: 0, y: 0, width: px, height: px))
    NSGraphicsContext.restoreGraphicsState()
    guard let png = resized.representation(using: .png, properties: [:]) else { exit(3) }
    try? png.write(to: iconset.appendingPathComponent(name))
}

let master = draw(canvas: 1024)
for (points, scales) in [(16, [1, 2]), (32, [1, 2]), (128, [1, 2]), (256, [1, 2]), (512, [1, 2])] {
    for scale in scales {
        let px = points * scale
        let suffix = scale == 1 ? "" : "@2x"
        write(master, px: px, name: "icon_\(points)x\(points)\(suffix).png")
    }
}
print("iconset written to \(iconset.path)")
