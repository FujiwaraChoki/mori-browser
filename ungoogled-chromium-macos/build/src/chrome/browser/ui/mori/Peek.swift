import SwiftUI
import AppKit

// MARK: - Store coordination

extension BrowserStore {
    /// Open a URL in a transient Peek overlay (Little Arc-style) without adding
    /// it to any space.
    func peek(url rawURL: String) {
        let resolved = URLInterpreter.resolve(rawURL, settings: settings)
        if BrowserURLPolicy.isPrivilegedURL(resolved),
           !confirmPrivilegedNavigation(resolved, source: "Peek") {
            ToastCenter.shared.show("Blocked internal URL", icon: "lock", style: .warning)
            return
        }
        guard BrowserURLPolicy.isWebURL(resolved) || BrowserURLPolicy.isPrivilegedURL(resolved) else {
            ToastCenter.shared.show("Nothing to peek", icon: "eye", style: .warning)
            return
        }
        let target = MoriURLRewriter.rewrite(resolved)
        let tab = BrowserTab(url: target, title: "Peek")
        tab.onRequestNewTab = { [weak self] u in self?.newTab(url: u) }
        tab.realize()
        tab.markAccessed()
        if let existing = peekTab { existing.close() }
        withAnimation(Motion.reveal) { peekTab = tab }
    }

    /// Peek the clipboard's URL if it holds one, else the current page — the
    /// classic "I just copied a link, let me glance at it" flow.
    func peekFromClipboardOrCurrent() {
        if let clip = NSPasteboard.general.string(forType: .string),
           URLInterpreter.resolvesAsAddress(clip) {
            peek(url: clip)
            return
        }
        if let url = selectedTab?.urlString, !url.isEmpty, url != "about:blank" {
            peek(url: url)
        } else {
            ToastCenter.shared.show("Nothing to peek", icon: "eye", style: .warning)
        }
    }

    func closePeek() {
        guard let tab = peekTab else { return }
        withAnimation(Motion.reveal) { peekTab = nil }
        tab.close()
    }

    /// Promote the peeked page into a real tab in the active context.
    func promotePeek() {
        guard let tab = peekTab else { return }
        withAnimation(Motion.reveal) { peekTab = nil }
        tab.onMetadataChanged = { [weak self] _ in self?.scheduleSessionSave() }
        tabs.append(tab)
        addToActiveContext(tab.id)
        selectTab(tab.id)
        scheduleSessionSave()
    }
}

// MARK: - Overlay

/// The Peek overlay: a centered floating card hosting a single transient tab,
/// with controls to promote it to a real tab or dismiss it.
struct PeekOverlay: View {
    @ObservedObject var store: BrowserStore

    var body: some View {
        ZStack {
            if let tab = store.peekTab {
                Color.black.opacity(0.32)
                    .ignoresSafeArea()
                    .onTapGesture { store.closePeek() }
                    .transition(.opacity)

                PeekCard(store: store, tab: tab)
                    .transition(.scale(scale: 0.96).combined(with: .opacity))
            }
        }
        .animation(Motion.reveal, value: store.peekTab != nil)
    }
}

private struct PeekCard: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        GeometryReader { geo in
            let width = min(geo.size.width * 0.74, 1000)
            let height = min(geo.size.height * 0.82, 820)
            VStack(spacing: 0) {
                header
                Hairline().opacity(0.5)
                PeekWebHost(tab: tab, cornerRadius: 0)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(width: width, height: height)
            .background(
                RoundedRectangle(cornerRadius: Radius.window, style: .continuous)
                    .fill(p.card.color)
            )
            .clipShape(RoundedRectangle(cornerRadius: Radius.window, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Radius.window, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.7), lineWidth: 1)
            )
            .elevation(.overlay, scheme)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private var header: some View {
        HStack(spacing: 9) {
            Favicon(icon: tab.faviconURL, page: tab.urlString,
                    image: tab.faviconImage, size: 15)
            VStack(alignment: .leading, spacing: 1) {
                Text(tab.title.isEmpty ? "Peek" : tab.title)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(prettyURL)
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 8)

            Button {
                store.copyURL(of: tab.id)
                ToastCenter.shared.show("Link copied", icon: "link", style: .success)
            } label: {
                Icon(name: "link", size: 14)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 26, height: 26)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Copy link")

            Button { store.promotePeek() } label: {
                Label("Open in Space", systemImage: "arrow.up.forward.app")
                    .font(Typography.ui(Typography.base, weight: .medium))
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)

            Button { store.closePeek() } label: {
                Icon(name: "xmark", size: 13, weight: .bold)
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 26, height: 26)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Close peek")
        }
        .padding(.horizontal, 12)
        .frame(height: 46)
    }

    private var prettyURL: String {
        guard let u = URL(string: tab.urlString) else { return tab.urlString }
        return (u.host ?? "") + u.path
    }
}

/// Hosts a single transient tab's live CEF view inside the Peek card.
private struct PeekWebHost: NSViewRepresentable {
    @ObservedObject var tab: BrowserTab
    var cornerRadius: CGFloat = 0

    func makeNSView(context: Context) -> NSView {
        let container = NSView()
        container.wantsLayer = true
        container.layer?.cornerCurve = .continuous
        container.layer?.cornerRadius = cornerRadius
        container.layer?.masksToBounds = cornerRadius > 0
        return container
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        let view = tab.realize()
        if view.superview !== nsView {
            view.removeFromSuperview()
            view.frame = nsView.bounds
            view.autoresizingMask = [.width, .height]
            nsView.addSubview(view)
        }
        view.isHidden = false
        view.setWebWindowVisible(true)
        view.setPageHidden(false)
    }
}
