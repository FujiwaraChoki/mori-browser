import SwiftUI
import AppKit

// MARK: - Store coordination & capture

extension BrowserStore {
    /// Arm the drag-to-select region capture overlay.
    func startRegionCapture() {
        guard !captureMode else { return }
        // Other transient overlays would be captured/obscured; clear them.
        dismissWebContextMenu()
        withAnimation(Motion.snappy) { captureMode = true }
        ToastCenter.shared.show("Drag to capture · Esc to cancel",
                                icon: "camera.viewfinder", style: .info, duration: 3)
    }

    func cancelRegionCapture() {
        guard captureMode else { return }
        withAnimation(Motion.snappy) { captureMode = false }
    }

    /// Called by the overlay with the selected rect (points, top-left in the
    /// window's content space).
    func finishRegionCapture(_ rect: CGRect) {
        withAnimation(Motion.snappy) { captureMode = false }
        guard rect.width > 4, rect.height > 4 else { return }
        // Let SwiftUI tear the overlay down before grabbing the window, so the
        // dim/selection chrome isn't baked into the screenshot.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.07) { [weak self] in
            self?.performRegionCapture(rect)
        }
    }

    private func performRegionCapture(_ rect: CGRect) {
        guard let window = captureWindow(),
              let full = Self.captureWindowImage(window) else {
            ToastCenter.shared.show("Couldn't capture the window", icon: "xmark", style: .warning)
            return
        }
        let scale = window.backingScaleFactor
        let px = CGRect(x: rect.minX * scale, y: rect.minY * scale,
                        width: rect.width * scale, height: rect.height * scale)
            .intersection(CGRect(x: 0, y: 0, width: full.width, height: full.height))
        guard !px.isNull, px.width > 1, px.height > 1,
              let cropped = full.cropping(to: px) else { return }
        Self.deliver(cropped)
    }

    /// Capture just the active tab's visible web content (the viewport).
    func captureVisibleArea() {
        guard let tab = selectedTab, tab.hasRealized else {
            ToastCenter.shared.show("Nothing to capture", icon: "camera", style: .warning)
            return
        }
        let view = tab.browserView
        guard let window = view.window, let full = Self.captureWindowImage(window) else {
            ToastCenter.shared.show("Couldn't capture the page", icon: "xmark", style: .warning)
            return
        }
        let scale = window.backingScaleFactor
        let contentH = window.contentView?.bounds.height ?? window.frame.height
        // Web view bounds → window coords (bottom-left) → top-left points.
        let inWindow = view.convert(view.bounds, to: nil)
        let topLeft = CGRect(x: inWindow.minX, y: contentH - inWindow.maxY,
                             width: inWindow.width, height: inWindow.height)
        let px = CGRect(x: topLeft.minX * scale, y: topLeft.minY * scale,
                        width: topLeft.width * scale, height: topLeft.height * scale)
            .intersection(CGRect(x: 0, y: 0, width: full.width, height: full.height))
        guard !px.isNull, let cropped = full.cropping(to: px) else { return }
        Self.deliver(cropped)
    }

    private func captureWindow() -> NSWindow? {
        selectedTab?.browserView.window ?? NSApp.keyWindow ?? NSApp.mainWindow
    }

    /// Grab the composited image of our own window (no screen-recording
    /// permission needed for one's own windows).
    private static func captureWindowImage(_ window: NSWindow) -> CGImage? {
        let windowID = CGWindowID(window.windowNumber)
        guard windowID != 0 else { return nil }
        return CGWindowListCreateImage(
            .null,
            .optionIncludingWindow,
            windowID,
            [.boundsIgnoreFraming, .bestResolution])
    }

    /// Copy the capture to the clipboard and save a PNG to the Desktop.
    private static func deliver(_ cg: CGImage) {
        let image = NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.writeObjects([image])

        var savedNote = "copied"
        let rep = NSBitmapImageRep(cgImage: cg)
        if let data = rep.representation(using: .png, properties: [:]) {
            let fm = FileManager.default
            let dir = fm.urls(for: .desktopDirectory, in: .userDomainMask).first
                ?? fm.urls(for: .downloadsDirectory, in: .userDomainMask).first
                ?? fm.temporaryDirectory
            let formatter = DateFormatter()
            formatter.dateFormat = "yyyy-MM-dd 'at' HH.mm.ss"
            let name = "Mori Shot \(formatter.string(from: Date())).png"
            if (try? data.write(to: dir.appendingPathComponent(name))) != nil {
                savedNote = "copied & saved to Desktop"
            }
        }
        ToastCenter.shared.show("Screenshot \(savedNote)", icon: "camera", style: .success)
    }
}

// MARK: - Region selection overlay
//
// AppKit-hosted (like LauncherOverlay) so the selection surface sits above the
// live web view and actually receives the drag — a plain SwiftUI `.overlay`
// would render behind the CEF content and never see the gesture.

struct CaptureOverlay: NSViewRepresentable {
    @ObservedObject var store: BrowserStore

    func makeNSView(context: Context) -> CaptureContainerView {
        let view = CaptureContainerView()
        view.update(store: store)
        return view
    }

    func updateNSView(_ nsView: CaptureContainerView, context: Context) {
        nsView.update(store: store)
    }
}

final class CaptureContainerView: NSView {
    private var hosting: NSHostingView<AnyView>?
    private weak var store: BrowserStore?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        let host = NSHostingView(rootView: AnyView(EmptyView()))
        host.frame = bounds
        host.autoresizingMask = [.width, .height]
        addSubview(host)
        hosting = host
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var isFlipped: Bool { true }

    func update(store: BrowserStore) {
        self.store = store
        rebuild()
    }

    private func rebuild() {
        guard let store else { return }
        hosting?.rootView = AnyView(
            Group {
                if store.captureMode {
                    CaptureSelectionView(store: store)
                        .frame(width: max(bounds.width, 1), height: max(bounds.height, 1))
                }
            }
        )
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard store?.captureMode == true else { return nil }
        return super.hitTest(point)
    }

    override func layout() {
        super.layout()
        hosting?.frame = bounds
        rebuild()
    }
}

private struct CaptureSelectionView: View {
    @ObservedObject var store: BrowserStore
    @State private var start: CGPoint?
    @State private var current: CGPoint?

    var body: some View {
        GeometryReader { _ in
            ZStack {
                    Color.black.opacity(0.28).ignoresSafeArea()
                    if let rect = selectionRect {
                        // Punch-through highlight of the chosen region.
                        Rectangle()
                            .fill(Color.white.opacity(0.10))
                            .frame(width: rect.width, height: rect.height)
                            .overlay(
                                Rectangle().strokeBorder(Color.white.opacity(0.95), lineWidth: 1.5)
                            )
                            .position(x: rect.midX, y: rect.midY)
                    }
                    if selectionRect == nil {
                        Text("Drag to capture a region")
                            .font(Typography.ui(13, weight: .medium))
                            .foregroundStyle(.white.opacity(0.9))
                            .padding(.horizontal, 12)
                            .padding(.vertical, 7)
                            .background(.black.opacity(0.45), in: Capsule())
                    }
                }
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 2, coordinateSpace: .local)
                        .onChanged { v in
                            if start == nil { start = v.startLocation }
                            current = v.location
                        }
                        .onEnded { v in
                            let rect = Self.rect(from: start ?? v.startLocation, to: v.location)
                            start = nil
                            current = nil
                            store.finishRegionCapture(rect)
                        }
                )
        }
    }

    private var selectionRect: CGRect? {
        guard let start, let current else { return nil }
        return Self.rect(from: start, to: current)
    }

    private static func rect(from a: CGPoint, to b: CGPoint) -> CGRect {
        CGRect(x: min(a.x, b.x), y: min(a.y, b.y),
               width: abs(a.x - b.x), height: abs(a.y - b.y))
    }
}
