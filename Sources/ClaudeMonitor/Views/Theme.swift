import AppKit
import SwiftUI

/// Claude Code's terminal palette, adapting to light and dark appearance.
enum Theme {
    static let claude = Color(red: 215 / 255, green: 119 / 255, blue: 87 / 255)
    static let text = adaptive(light: (20, 20, 20), dark: (235, 235, 235))
    static let dim = adaptive(light: (102, 102, 102), dark: (153, 153, 153))
    static let subtle = adaptive(light: (190, 190, 190), dark: (75, 75, 75))
    static let background = adaptive(light: (252, 252, 250), dark: (22, 22, 22))
    static let success = adaptive(light: (44, 122, 57), dark: (78, 186, 101))
    static let warning = adaptive(light: (150, 108, 30), dark: (255, 193, 7))

    static func mono(_ size: CGFloat = 12, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }

    private static func adaptive(light: (Int, Int, Int), dark: (Int, Int, Int)) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let (r, g, b) = appearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark : light
            return NSColor(srgbRed: CGFloat(r) / 255, green: CGFloat(g) / 255, blue: CGFloat(b) / 255, alpha: 1)
        })
    }
}

/// Rounded box like Claude Code's welcome banner.
struct TermBox<Content: View>: View {
    var border: Color = Theme.claude
    @ViewBuilder let content: Content

    var body: some View {
        content
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(RoundedRectangle(cornerRadius: 6).stroke(border, lineWidth: 1))
    }
}

/// `⏺ Title` followed by an indented `⎿` result block, as Claude Code renders tool calls.
struct TermSection<Content: View>: View {
    let title: String
    var bullet: Color = Theme.claude
    var trailing: String?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text("⏺").foregroundStyle(bullet)
                Text(title).fontWeight(.semibold).foregroundStyle(Theme.text)
                Spacer()
                if let trailing {
                    Text(trailing).foregroundStyle(Theme.dim)
                }
            }
            HStack(alignment: .top, spacing: 6) {
                Text("⎿").foregroundStyle(Theme.dim)
                VStack(alignment: .leading, spacing: 2) { content }
            }
            .padding(.leading, 8)
        }
    }
}

/// `████████░░░░` progress bar drawn with block characters.
struct TextBar: View {
    let fraction: Double
    var width = 26

    var body: some View {
        let filled = Int((min(max(fraction, 0), 1) * Double(width)).rounded())
        HStack(spacing: 0) {
            Text(String(repeating: "█", count: filled)).foregroundStyle(Theme.claude)
            Text(String(repeating: "░", count: width - filled)).foregroundStyle(Theme.subtle)
        }
    }
}

/// Claude Code's thinking spinner. Animates only while `active`.
struct Spinner: View {
    let active: Bool
    private static let frames = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"]

    var body: some View {
        if active {
            TimelineView(.periodic(from: .now, by: 0.12)) { context in
                let i = Int(context.date.timeIntervalSinceReferenceDate / 0.12) % Self.frames.count
                glyph(Self.frames[i])
            }
        } else {
            glyph("✻")
        }
    }

    private func glyph(_ s: String) -> some View {
        Text(s).foregroundStyle(Theme.claude).frame(width: 14)
    }
}
