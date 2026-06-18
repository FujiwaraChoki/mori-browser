import SwiftUI
import AppKit

// MARK: - Model

/// What the user right-clicked on in the page.
struct LinkImageContextTarget {
    var linkURL: String?
    var linkText: String = ""
    var imageURL: String?
    var selection: String = ""

    var hasContent: Bool { linkURL != nil || imageURL != nil }
}

/// A pending custom context menu: its target plus where to anchor it (top-left,
/// in the root view's coordinate space).
struct WebContextMenuRequest: Identifiable {
    let id = UUID()
    let target: LinkImageContextTarget
    let location: CGPoint
    /// The raw right-click point in window coordinates (AppKit, bottom-left
    /// origin). Used to drive Chromium's native copy/save-image at that point.
    let windowPoint: CGPoint
}

// MARK: - Page scripts

enum WebContextScripts {
    /// Installs a capturing contextmenu listener. When the cursor is over a link
    /// or image it records the target and `preventDefault()`s so Chrome's native
    /// menu is suppressed and Mori's custom menu can take over.
    static let listener = """
    (() => {
      if (window.__moriCtxInstalled) return;
      window.__moriCtxInstalled = true;
      window.__moriCtxSeq = 0;
      const closestLink = (el) => {
        while (el && el.nodeType === 1) {
          if (el.tagName === 'A' && el.href) return el;
          el = el.parentElement;
        }
        return null;
      };
      const bgImage = (el) => {
        try {
          const s = getComputedStyle(el).backgroundImage;
          const m = s && s.match(/url\\(["']?(.*?)["']?\\)/);
          return m ? m[1] : '';
        } catch (e) { return ''; }
      };
      const imageFor = (el) => {
        let node = el;
        while (node && node.nodeType === 1) {
          if (node.tagName === 'IMG' && (node.currentSrc || node.src)) {
            return node.currentSrc || node.src;
          }
          node = node.parentElement;
        }
        return bgImage(el) || '';
      };
      document.addEventListener('contextmenu', (e) => {
        const link = closestLink(e.target);
        const image = imageFor(e.target);
        const selection = String(window.getSelection ? getSelection() : '').trim();
        if (link || image) {
          window.__moriCtx = {
            seq: ++window.__moriCtxSeq,
            link: link ? link.href : '',
            linkText: link ? (link.innerText || link.textContent || '').trim().slice(0, 140) : '',
            image: image || '',
            selection: selection
          };
          e.preventDefault();
          e.stopPropagation();
        } else {
          window.__moriCtx = null;
        }
      }, true);
    })();
    """

    /// Reads and clears the last captured target.
    static let read = """
    (() => { const c = window.__moriCtx; window.__moriCtx = null; return c || null; })();
    """
}

// MARK: - Store coordination & actions

extension BrowserStore {
    /// Handle a right-click anywhere in the window: if it landed on a page link
    /// or image, present Mori's custom menu at the cursor.
    func handleWebRightClick(_ event: NSEvent) {
        guard let tab = selectedTab, tab.hasRealized else { return }
        let window = event.window
        let locationInWindow = event.locationInWindow
        Task { @MainActor in
            // Poll briefly: the page's contextmenu handler stashes the payload a
            // beat after the native right-mouse-down reaches us.
            var found: LinkImageContextTarget?
            for _ in 0..<4 {
                try? await Task.sleep(nanoseconds: 25_000_000)
                if let target = await tab.readContextMenuTarget() {
                    found = target
                    break
                }
            }
            guard let target = found else { return }
            let height = window?.contentView?.bounds.height
                ?? window?.frame.height ?? 0
            let point = CGPoint(x: locationInWindow.x, y: height - locationInWindow.y)
            withAnimation(Motion.snappy) {
                self.contextMenu = WebContextMenuRequest(
                    target: target, location: point, windowPoint: locationInWindow)
            }
        }
    }

    func dismissWebContextMenu() {
        guard contextMenu != nil else { return }
        withAnimation(Motion.snappy) { contextMenu = nil }
    }

    // Link actions

    func ctxOpenLink(_ url: String, inBackground: Bool) {
        guard let safeURL = resolvePageDerivedNavigationURL(url, source: "A page link") else {
            dismissWebContextMenu()
            return
        }
        newTab(url: safeURL, select: !inBackground)
        dismissWebContextMenu()
    }

    func ctxPeekLink(_ url: String) {
        guard let safeURL = resolvePageDerivedNavigationURL(url, source: "A page link") else {
            dismissWebContextMenu()
            return
        }
        peek(url: safeURL)
        dismissWebContextMenu()
    }

    /// Open the link in a chosen space (creating the tab there and switching to it).
    func ctxOpenLink(_ url: String, inContext id: BrowserContext.ID) {
        guard let safeURL = resolvePageDerivedNavigationURL(url, source: "A page link") else {
            dismissWebContextMenu()
            return
        }
        let tab = newTab(url: safeURL, select: false)
        moveTab(tab.id, toContext: id, activate: true)
        dismissWebContextMenu()
    }

    // Image actions

    func ctxOpenImage(_ url: String) {
        guard let safeURL = resolvePageDerivedNavigationURL(url, source: "A page image") else {
            dismissWebContextMenu()
            return
        }
        newTab(url: safeURL, select: true)
        dismissWebContextMenu()
    }

    func ctxSaveImage(_ url: String) {
        let tab = selectedTab
        let point = contextMenu?.windowPoint ?? .zero
        dismissWebContextMenu()
        let ok = tab?.saveImage(url: url, at: point) ?? false
        ToastCenter.shared.show(ok ? "Saving image…" : "Couldn't save that image",
                                icon: ok ? "square.and.arrow.down" : "xmark",
                                style: ok ? .success : .warning)
    }

    func ctxCopyImage(_ url: String) {
        let tab = selectedTab
        let point = contextMenu?.windowPoint
        dismissWebContextMenu()
        if let point, tab?.copyImage(at: point) == true {
            ToastCenter.shared.show("Image copied", icon: "doc.on.doc", style: .success)
        } else {
            // Fall back to copying the address when there's no live image.
            copyToPasteboard(url)
            ToastCenter.shared.show("Copied image address", icon: "link", style: .success)
        }
    }

    func ctxSearchImage(_ url: String) {
        guard let explicitURL = BrowserURLPolicy.explicitURL(url),
              BrowserURLPolicy.isWebURL(explicitURL) else {
            dismissWebContextMenu()
            ToastCenter.shared.show("Blocked non-web image search", icon: "lock", style: .warning)
            return
        }
        let encoded = explicitURL.addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)
            ?? explicitURL
        newTab(url: "https://lens.google.com/uploadbyurl?url=\(encoded)", select: true)
        dismissWebContextMenu()
    }

    // Shared

    func ctxCopy(_ string: String, message: String) {
        copyToPasteboard(string)
        ToastCenter.shared.show(message, icon: "doc.on.doc", style: .success)
        dismissWebContextMenu()
    }

    private func copyToPasteboard(_ string: String) {
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(string, forType: .string)
    }
}

// MARK: - Right-click catcher

/// Installs an app-local right-mouse-down monitor so Mori can intercept page
/// right-clicks. Passive: it always returns the event so chrome context menus
/// and the renderer still receive it.
struct WebRightClickCatcher: NSViewRepresentable {
    let store: BrowserStore

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        context.coordinator.install(store: store)
        return NSView()
    }

    func updateNSView(_ nsView: NSView, context: Context) {
        context.coordinator.store = store
    }

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.remove()
    }

    final class Coordinator {
        weak var store: BrowserStore?
        private var monitor: Any?

        func install(store: BrowserStore) {
            self.store = store
            monitor = NSEvent.addLocalMonitorForEvents(matching: .rightMouseDown) { [weak self] event in
                self?.store?.handleWebRightClick(event)
                return event
            }
        }

        func remove() {
            if let monitor { NSEvent.removeMonitor(monitor) }
            monitor = nil
        }
    }
}

// MARK: - Overlay

struct WebContextMenuOverlay: View {
    @ObservedObject var store: BrowserStore

    var body: some View {
        GeometryReader { geo in
            if let request = store.contextMenu {
                ZStack(alignment: .topLeading) {
                    // Invisible catcher: a click anywhere dismisses.
                    Color.black.opacity(0.001)
                        .contentShape(Rectangle())
                        .onTapGesture { store.dismissWebContextMenu() }

                    WebContextMenuCard(store: store, target: request.target)
                        .modifier(MenuPlacement(point: request.location, bounds: geo.size))
                }
                .transition(.opacity)
            }
        }
    }
}

/// Positions the menu at the click point, clamped to stay fully on-screen.
private struct MenuPlacement: ViewModifier {
    let point: CGPoint
    let bounds: CGSize
    @State private var size: CGSize = .zero

    func body(content: Content) -> some View {
        let x = min(max(8, point.x), max(8, bounds.width - size.width - 8))
        let y = min(max(8, point.y), max(8, bounds.height - size.height - 8))
        content
            .background(
                GeometryReader { g in
                    Color.clear
                        .onAppear { size = g.size }
                        .onChange(of: g.size) { _, s in size = s }
                }
            )
            .offset(x: x, y: y)
    }
}

private struct WebContextMenuCard: View {
    @Environment(\.colorScheme) private var scheme
    @ObservedObject var store: BrowserStore
    let target: LinkImageContextTarget
    @Environment(\.palette) private var p

    private enum Page { case main, spaces }
    @State private var page: Page = .main

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            switch page {
            case .main: mainItems
            case .spaces: spaceItems
            }
        }
        .padding(5)
        .frame(width: 240)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.popover.color)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
        .elevation(.popover, scheme)
    }

    @ViewBuilder
    private var mainItems: some View {
        if let link = target.linkURL {
            CtxHeader(text: target.linkText.isEmpty ? link : target.linkText)
            CtxRow(icon: "rectangle.badge.plus", title: "Open Link in New Tab") {
                store.ctxOpenLink(link, inBackground: false)
            }
            CtxRow(icon: "rectangle.on.rectangle", title: "Open in Background Tab") {
                store.ctxOpenLink(link, inBackground: true)
            }
            CtxRow(icon: "eye", title: "Open in Peek") {
                store.ctxPeekLink(link)
            }
            CtxRow(icon: "square.grid.2x2", title: "Open Link in Space", trailing: "chevron.right") {
                page = .spaces
            }
            CtxRow(icon: "link", title: "Copy Link Address") {
                store.ctxCopy(link, message: "Link copied")
            }
        }

        if let image = target.imageURL {
            if target.linkURL != nil { CtxDivider() }
            CtxRow(icon: "photo", title: "Open Image in New Tab") {
                store.ctxOpenImage(image)
            }
            CtxRow(icon: "square.and.arrow.down", title: "Save Image…") {
                store.ctxSaveImage(image)
            }
            CtxRow(icon: "doc.on.doc", title: "Copy Image") {
                store.ctxCopyImage(image)
            }
            CtxRow(icon: "link", title: "Copy Image Address") {
                store.ctxCopy(image, message: "Image address copied")
            }
            CtxRow(icon: "magnifyingglass", title: "Search Image with Google") {
                store.ctxSearchImage(image)
            }
        }
    }

    @ViewBuilder
    private var spaceItems: some View {
        CtxRow(icon: "chevron.left", title: "Back") { page = .main }
        CtxDivider()
        if let link = target.linkURL {
            ForEach(store.contexts) { context in
                CtxRow(icon: "circle.fill", title: context.name) {
                    store.ctxOpenLink(link, inContext: context.id)
                }
            }
        }
    }
}

private struct CtxHeader: View {
    let text: String
    @Environment(\.palette) private var p

    var body: some View {
        Text(text)
            .font(Typography.ui(Typography.small, weight: .medium))
            .foregroundStyle(p.mutedForeground.color)
            .lineLimit(1)
            .truncationMode(.middle)
            .padding(.horizontal, 9)
            .padding(.top, 4)
            .padding(.bottom, 3)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct CtxDivider: View {
    @Environment(\.palette) private var p
    var body: some View {
        Rectangle()
            .fill(p.border.color.opacity(0.5))
            .frame(height: 1)
            .padding(.vertical, 3)
            .padding(.horizontal, 4)
    }
}

private struct CtxRow: View {
    let icon: String
    let title: String
    var trailing: String? = nil
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                Icon(name: icon, size: 13)
                    .foregroundStyle(hovering ? p.primaryForeground.color : p.mutedForeground.color)
                    .frame(width: 16)
                Text(title)
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(hovering ? p.primaryForeground.color : p.popoverForeground.color)
                    .lineLimit(1)
                Spacer(minLength: 6)
                if let trailing {
                    Icon(name: trailing, size: 10)
                        .foregroundStyle(hovering ? p.primaryForeground.color : p.mutedForeground.color)
                }
            }
            .padding(.horizontal, 9)
            .frame(height: 30)
            .background(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .fill(hovering ? p.primary.color : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.snappy, value: hovering)
    }
}
