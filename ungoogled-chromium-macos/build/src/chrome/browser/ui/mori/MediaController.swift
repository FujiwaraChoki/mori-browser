import SwiftUI

/// A snapshot of one page's primary media, parsed from the injected agent.
struct MediaState: Equatable {
    var hasMedia = false
    var playing = false
    var title = ""
    var artist = ""
    var artwork = ""
    var position: Double = 0
    var duration: Double = 0
    var muted = false
    var isVideo = false
    var inPiP = false
    var canPiP = false
    var browserId: Int = 0
}

/// Aggregates media state broadcast by every tab's injected agent and exposes
/// playback controls for the sidebar player. The "active" source is whichever
/// tab is most recently playing.
final class MediaController: ObservableObject {
    @Published private(set) var state = MediaState()

    /// Resolves a CEF browser id to its owning tab (wired by the store).
    var resolveTab: ((Int) -> BrowserTab?)?

    private var byBrowser: [Int: MediaState] = [:]
    private var order: [Int] = []           // browser ids, most-recent last
    private var observer: NSObjectProtocol?
    private var wasInPiP = false

    init() {
        observer = NotificationCenter.default.addObserver(
            forName: Notification.Name("MoriMediaUpdated"),
            object: nil, queue: .main
        ) { [weak self] note in
            self?.handle(note)
        }
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    // MARK: Ingest

    private func handle(_ note: Notification) {
        guard let info = note.userInfo,
              let bid = info["browserId"] as? Int,
              let json = info["json"] as? String,
              let data = json.data(using: .utf8),
              let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
        else { return }

        var s = MediaState()
        s.browserId = bid
        s.hasMedia = obj["hasMedia"] as? Bool ?? false
        if s.hasMedia {
            s.playing = obj["playing"] as? Bool ?? false
            s.title = (obj["title"] as? String ?? "").trimmingCharacters(in: .whitespaces)
            s.artist = obj["artist"] as? String ?? ""
            s.artwork = obj["artwork"] as? String ?? ""
            s.position = obj["position"] as? Double ?? 0
            s.duration = obj["duration"] as? Double ?? 0
            s.muted = obj["muted"] as? Bool ?? false
            s.isVideo = obj["isVideo"] as? Bool ?? false
            s.inPiP = obj["inPiP"] as? Bool ?? false
            s.canPiP = obj["canPiP"] as? Bool ?? false
        }

        if s.hasMedia {
            byBrowser[bid] = s
            order.removeAll { $0 == bid }
            order.append(bid)
        } else {
            byBrowser.removeValue(forKey: bid)
            order.removeAll { $0 == bid }
        }
        recomputeActive()
    }

    private func recomputeActive() {
        let candidates = order.reversed().compactMap { byBrowser[$0] }
        let chosen = candidates.first(where: { $0.playing }) ?? candidates.first ?? MediaState()
        if chosen != state { state = chosen }

        // When any source enters PiP, round the engine-created PiP window.
        let inPiP = byBrowser.values.contains { $0.inPiP }
        if inPiP && !wasInPiP {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) {
                PiPWindowStyler.roundPiPWindows()
            }
            // A second pass in case Chromium finishes styling the window late.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) {
                PiPWindowStyler.roundPiPWindows()
            }
        }
        wasInPiP = inPiP
    }

    // MARK: Controls

    private func command(_ action: String, _ value: Double = 0) {
        guard state.hasMedia, let tab = resolveTab?(state.browserId) else { return }
        tab.browserView.sendMediaCommand(action, value: value)
    }

    var hasMedia: Bool { state.hasMedia }

    func togglePlay() { command(state.playing ? "pause" : "play") }
    func skipForward() { command("seekBy", 10) }
    func skipBack() { command("seekBy", -10) }
    func seek(to seconds: Double) { command("seek", seconds) }
    func toggleMute() { command("mute") }
    func togglePiP() { command("pip") }

    /// Bring the tab that owns the active media to the foreground.
    func revealOwningTab(in store: BrowserStore) {
        guard let tab = resolveTab?(state.browserId) else { return }
        store.selectTab(tab.id)
    }
}
