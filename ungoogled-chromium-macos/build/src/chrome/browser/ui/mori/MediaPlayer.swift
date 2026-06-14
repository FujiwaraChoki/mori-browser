import SwiftUI

/// The dynamic sidebar media player. A single compact row: artwork (with
/// Picture-in-Picture on hover), title + channel, and skip ±10s / play-pause
/// transport — over a thin scrubbable progress line.
/// Appears only while a tab is playing/holding media; animates in and out.
struct MediaPlayerStrip: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var media: MediaController
    @Environment(\.palette) private var p

    @State private var scrubbing = false
    @State private var scrubValue: Double = 0
    @State private var hoveringArt = false

    private var s: MediaState { media.state }

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 9) {
                artwork

                VStack(alignment: .leading, spacing: 1) {
                    Text(s.title.isEmpty ? "Playing" : s.title)
                        .font(Typography.ui(Typography.label, weight: .medium))
                        .foregroundStyle(p.sidebarForeground.color)
                        .lineLimit(1)
                    Text(s.artist)
                        .font(Typography.ui(Typography.small))
                        .foregroundStyle(p.mutedForeground.color)
                        .lineLimit(1)
                }

                Spacer(minLength: 4)

                HStack(spacing: 3) {
                    PlayerButton(systemName: "gobackward.10") { media.skipBack() }
                    PlayerButton(systemName: s.playing ? "pause.fill" : "play.fill",
                                 prominent: true) { media.togglePlay() }
                    PlayerButton(systemName: "goforward.10") { media.skipForward() }
                }
            }

            scrubber
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.sidebarAccent.color.opacity(p.sidebarAccent.a == 1 ? 0.7 : 1))
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.sidebarBorder.color.opacity(0.5), lineWidth: 1)
        )
        .padding(.horizontal, 8)
        .padding(.bottom, 6)
    }

    // MARK: Pieces

    private var artwork: some View {
        Group {
            if let url = artURL {
                AsyncImage(url: url) { phase in
                    if case .success(let img) = phase {
                        img.resizable().aspectRatio(contentMode: .fill)
                    } else {
                        artFallback
                    }
                }
            } else {
                artFallback
            }
        }
        .frame(width: 38, height: 38)
        .clipShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
        .overlay(pipOverlay)
        .overlay(
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .strokeBorder(p.sidebarBorder.color.opacity(0.4), lineWidth: 0.5)
        )
        .contentShape(RoundedRectangle(cornerRadius: 7, style: .continuous))
        .onTapGesture { media.revealOwningTab(in: store) }
        .onHover { hoveringArt = $0 }
    }

    /// PiP affordance revealed when hovering the thumbnail.
    @ViewBuilder private var pipOverlay: some View {
        if (s.canPiP || s.inPiP), hoveringArt || s.inPiP {
            Button { media.togglePiP() } label: {
                ZStack {
                    RoundedRectangle(cornerRadius: 7, style: .continuous)
                        .fill(.black.opacity(s.inPiP ? 0.45 : 0.55))
                    Icon(name: s.inPiP ? "pip.exit" : "pip.enter", size: 16, weight: .medium)
                        .foregroundStyle(.white)
                }
            }
            .buttonStyle(.plain)
            .help("Picture in Picture")
            .transition(.opacity)
        }
    }

    private var artFallback: some View {
        ZStack {
            p.muted.color
            Icon(name: s.isVideo ? "play.rectangle.fill" : "music.note", size: 18, weight: .regular)
                .foregroundStyle(p.mutedForeground.color)
        }
    }

    private var artURL: URL? {
        if !s.artwork.isEmpty { return URL(string: s.artwork) }
        if let tab = media.resolveTab?(s.browserId), let f = tab.faviconURL {
            return URL(string: f)
        }
        return nil
    }

    private var scrubber: some View {
        GeometryReader { geo in
            let w = geo.size.width
            let frac = progressFraction
            ZStack(alignment: .leading) {
                Capsule().fill(p.foreground.color.opacity(0.12))
                    .frame(height: 3)
                Capsule().fill(p.primary.color)
                    .frame(width: max(0, min(w, w * frac)), height: 3)
                Circle().fill(p.primary.color)
                    .frame(width: scrubbing ? 11 : 8, height: scrubbing ? 11 : 8)
                    .offset(x: max(0, min(w, w * frac)) - (scrubbing ? 5.5 : 4))
                    .shadow(color: .black.opacity(0.15), radius: 1, y: 0.5)
            }
            .frame(height: 12)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { v in
                        scrubbing = true
                        scrubValue = max(0, min(1, v.location.x / w)) * max(s.duration, 0)
                    }
                    .onEnded { _ in
                        if s.duration > 0 { media.seek(to: scrubValue) }
                        scrubbing = false
                    }
            )
            .animation(Motion.state, value: scrubbing)
        }
        .frame(height: 12)
    }

    private var progressFraction: Double {
        guard s.duration > 0 else { return 0 }
        let pos = scrubbing ? scrubValue : s.position
        return max(0, min(1, pos / s.duration))
    }
}

/// A transport button. The whole transport renders from one SF Symbol family
/// at a single weight so the controls read as a uniform set; the prominent
/// play/pause gets a solid primary disc with a contrasting glyph.
private struct PlayerButton: View {
    let systemName: String
    var prominent: Bool = false
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: prominent ? 12 : 13, weight: .semibold))
                .foregroundStyle(foreground)
                .frame(width: prominent ? 28 : 26, height: prominent ? 28 : 26)
                .background(background)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }

    private var foreground: Color {
        prominent ? p.primaryForeground.color : p.sidebarForeground.color.opacity(hovering ? 1 : 0.8)
    }

    @ViewBuilder private var background: some View {
        if prominent {
            Circle().fill(p.primary.color.opacity(hovering ? 0.9 : 1))
        } else {
            Circle().fill(hovering ? p.foreground.color.opacity(0.08) : .clear)
        }
    }
}
