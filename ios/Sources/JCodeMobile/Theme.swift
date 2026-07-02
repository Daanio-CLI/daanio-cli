import SwiftUI

/// Design tokens. Dark, calm, terminal-native; mint accent for live state.
enum Theme {
    static let background = Color(hex: 0x0F0F14)
    static let surface = Color(hex: 0x1A1A1F)
    static let surfaceElevated = Color(hex: 0x242429)
    static let border = Color.white.opacity(0.08)
    static let mint = Color(hex: 0x4DD9A6)
    static let mintTint = Color(hex: 0x4DD9A6).opacity(0.15)
    static let textPrimary = Color.white.opacity(0.92)
    static let textSecondary = Color.white.opacity(0.55)
    static let textTertiary = Color.white.opacity(0.35)
    static let warning = Color(hex: 0xF59E0B)
    static let error = Color(hex: 0xD94D59)

    static func mono(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }

    /// Decorative icon font (SF Symbols) at a fixed point size.
    static func icon(_ size: CGFloat, weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight)
    }
}

extension Color {
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255.0,
            green: Double((hex >> 8) & 0xFF) / 255.0,
            blue: Double(hex & 0xFF) / 255.0
        )
    }
}

/// Runtime safe-area insets of the key window (zero when unavailable).
///
/// Home-button devices (iPhone SE class) report a zero bottom inset, so
/// edge-pinned chrome needs explicit breathing room there; Dynamic Island
/// devices already get it from the system insets.
@MainActor
enum SafeArea {
    static var top: CGFloat { insets.top }
    static var bottom: CGFloat { insets.bottom }

    /// Extra padding for chrome pinned to an edge with no system inset.
    static var compactTopPad: CGFloat { top < 24 ? 12 : 0 }
    static var compactBottomPad: CGFloat { bottom > 0 ? 0 : 12 }

    private static var insets: UIEdgeInsets {
        UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow?.safeAreaInsets }
            .first ?? .zero
    }
}

/// Card container used across screens.
struct Card<Content: View>: View {
    @ViewBuilder var content: Content

    var body: some View {
        content
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Theme.surface)
            .clipShape(RoundedRectangle(cornerRadius: 14))
            .overlay(
                RoundedRectangle(cornerRadius: 14)
                    .stroke(Theme.border, lineWidth: 1)
            )
    }
}
