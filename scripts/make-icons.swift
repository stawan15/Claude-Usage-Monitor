// Draws the app and tray icons (the ✻ glyph from Claude Code) as PNGs.
// Usage: swift scripts/make-icons.swift && bunx tauri icon assets/icon-source.png
import AppKit

func render(size: CGFloat, background: NSColor?, glyph: NSColor, glyphScale: CGFloat, to path: String) {
    let pixels = Int(size)
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                               bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let rect = NSRect(x: 0, y: 0, width: size, height: size)

    if let background {
        // macOS-style squircle-ish rounded square with a margin.
        let inset = size * 0.1
        let tile = rect.insetBy(dx: inset, dy: inset)
        background.setFill()
        NSBezierPath(roundedRect: tile, xRadius: tile.width * 0.225, yRadius: tile.width * 0.225).fill()
    }

    // Center the glyph's actual outline; text metrics put ✻ visibly off-center.
    let font = NSFont.systemFont(ofSize: size * glyphScale, weight: .bold) as CTFont
    var character: UniChar = 0x273B // ✻
    var cgGlyph: CGGlyph = 0
    let ctFont = CTFontCreateForString(font, "✻" as CFString, CFRange(location: 0, length: 1))
    CTFontGetGlyphsForCharacters(ctFont, &character, &cgGlyph, 1)
    let outline = CTFontCreatePathForGlyph(ctFont, cgGlyph, nil)!
    let box = outline.boundingBoxOfPath
    var move = CGAffineTransform(translationX: rect.midX - box.midX, y: rect.midY - box.midY)
    let context = NSGraphicsContext.current!.cgContext
    context.addPath(outline.copy(using: &move)!)
    context.setFillColor(glyph.cgColor)
    context.fillPath()

    NSGraphicsContext.restoreGraphicsState()
    try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: path))
}

let claude = NSColor(srgbRed: 215 / 255, green: 119 / 255, blue: 87 / 255, alpha: 1)
let dark = NSColor(srgbRed: 22 / 255, green: 22 / 255, blue: 22 / 255, alpha: 1)

render(size: 1024, background: dark, glyph: claude, glyphScale: 0.62, to: "assets/icon-source.png")
render(size: 64, background: nil, glyph: claude, glyphScale: 0.95, to: "src-tauri/icons/tray.png")
// macOS menu bar template: black + alpha only; the system tints it for light/dark menu bars.
render(size: 44, background: nil, glyph: .black, glyphScale: 0.95, to: "src-tauri/icons/tray-template.png")
