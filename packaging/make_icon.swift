// Draws the 1024x1024 app icon: one 4/4 bar of beats above "BPM".
// Usage: swiftc -o make_icon make_icon.swift && ./make_icon <output.png>

import AppKit
import ImageIO

let canvasSize = 1024
let colorSpace = CGColorSpaceCreateDeviceRGB()
let context = CGContext(
    data: nil, width: canvasSize, height: canvasSize, bitsPerComponent: 8, bytesPerRow: 0,
    space: colorSpace, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!

// Rounded tile on the standard macOS icon grid.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
context.saveGState()
context.addPath(CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil))
context.clip()
let background = CGGradient(
    colorsSpace: colorSpace,
    colors: [CGColor(red: 0.17, green: 0.12, blue: 0.25, alpha: 1), CGColor(red: 0.05, green: 0.05, blue: 0.08, alpha: 1)] as CFArray,
    locations: [0, 1])!
context.drawLinearGradient(background, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])
context.restoreGState()

// Four beats, the downbeat highlighted.
let accent = CGColor(red: 1.0, green: 0.18, blue: 0.58, alpha: 1)
let beatColor = CGColor(red: 1, green: 1, blue: 1, alpha: 0.88)
let barWidth: CGFloat = 92
let barGap: CGFloat = 58
let barsBaseline: CGFloat = 470
let barHeights: [CGFloat] = [300, 165, 230, 165]
let barsWidth = barWidth * 4 + barGap * 3
for (index, height) in barHeights.enumerated() {
    let x = 512 - barsWidth / 2 + CGFloat(index) * (barWidth + barGap)
    let bar = CGRect(x: x, y: barsBaseline, width: barWidth, height: height)
    context.addPath(CGPath(roundedRect: bar, cornerWidth: 26, cornerHeight: 26, transform: nil))
    context.setFillColor(index == 0 ? accent : beatColor)
    context.fillPath()
}

// "BPM" label.
let font = NSFont.systemFont(ofSize: 210, weight: .heavy)
let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: CGColor(red: 1, green: 1, blue: 1, alpha: 1)]
let line = CTLineCreateWithAttributedString(NSAttributedString(string: "BPM", attributes: attributes))
let textBounds = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
context.textPosition = CGPoint(x: 512 - textBounds.width / 2 - textBounds.minX, y: 190 - textBounds.minY)
CTLineDraw(line, context)

let output = URL(fileURLWithPath: CommandLine.arguments[1])
let destination = CGImageDestinationCreateWithURL(output as CFURL, "public.png" as CFString, 1, nil)!
CGImageDestinationAddImage(destination, context.makeImage()!, nil)
guard CGImageDestinationFinalize(destination) else { fatalError("Cannot write \(output.path)") }
