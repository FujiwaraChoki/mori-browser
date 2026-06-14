import AppKit
import SwiftUI

/// Rounds the corners of Chromium's Picture-in-Picture window, which ships with
/// square corners. The PiP window is engine-created, so we find it by its
/// floating window level (above normal) and clip its content layer.
enum PiPWindowStyler {
    static func roundPiPWindows(radius: CGFloat = 12) {
        for window in NSApp.windows {
            guard window.isVisible,
                  // PiP floats above the main window; the main window is .normal.
                  window.level.rawValue > NSWindow.Level.normal.rawValue,
                  // Never restyle our own SwiftUI-hosted chrome.
                  !(window.contentViewController is NSHostingController<RootView>),
                  let content = window.contentView
            else { continue }

            content.wantsLayer = true
            content.layer?.cornerRadius = radius
            content.layer?.cornerCurve = .continuous
            content.layer?.masksToBounds = true

            // Let the now-transparent corners show through, and refresh the
            // window shadow so it follows the rounded shape.
            window.isOpaque = false
            window.backgroundColor = .clear
            window.invalidateShadow()
        }
    }
}
