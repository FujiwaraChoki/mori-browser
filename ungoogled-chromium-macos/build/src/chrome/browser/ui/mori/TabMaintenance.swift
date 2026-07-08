import SwiftUI

/// Tab lifecycle maintenance: Arc-style auto-archiving of stale tabs and
/// memory-reclaiming auto-sleep of idle background tabs. A single low-frequency
/// timer drives both; thresholds come from `BrowserSettings`.
extension BrowserStore {
    /// How often the maintenance pass runs. Coarse on purpose — this is
    /// housekeeping, not something the user should ever feel.
    private static let maintenanceInterval: TimeInterval = 60

    func startTabMaintenance() {
        maintenanceTimer?.invalidate()
        let timer = Timer.scheduledTimer(withTimeInterval: Self.maintenanceInterval,
                                         repeats: true) { [weak self] _ in
            self?.runMaintenancePass()
        }
        timer.tolerance = 15
        maintenanceTimer = timer
    }

    /// Tabs that must never be touched by a sleep pass: the visible ones.
    private var sleepProtectedIDs: Set<BrowserTab.ID> {
        var ids = Set<BrowserTab.ID>()
        if let id = selectedTabID { ids.insert(id) }
        if let id = splitTabID { ids.insert(id) }
        return ids
    }

    /// Tabs that must never be auto-archived: the visible ones plus pinned
    /// tiles (favorites are permanent in Arc) and non-web pages.
    private var archiveProtectedIDs: Set<BrowserTab.ID> {
        var ids = sleepProtectedIDs
        for context in contexts { ids.formUnion(context.pinnedTabIDs) }
        for context in contexts {
            ids.formUnion(context.folders.flatMap(\.tabIDs))
        }
        return ids
    }

    private static func isArchivable(_ url: String) -> Bool {
        url.hasPrefix("http://") || url.hasPrefix("https://")
    }

    func runMaintenancePass() {
        let settings = self.settings
        let now = Date()
        let archiveCutoff: Date? = settings.autoArchiveHours > 0
            ? now.addingTimeInterval(-Double(settings.autoArchiveHours) * 3600)
            : nil
        let sleepCutoff: Date? = settings.autoSleepMinutes > 0
            ? now.addingTimeInterval(-Double(settings.autoSleepMinutes) * 60)
            : nil
        guard archiveCutoff != nil || sleepCutoff != nil else { return }

        var didSleepAny = false

        // Archive first: a tab past the archive horizon is closed (restorable),
        // so there's no point sleeping it. Snapshot the ids since archiving
        // mutates the tab list.
        if let archiveCutoff {
            let protected = archiveProtectedIDs
            let doomed = tabs.filter { tab in
                !protected.contains(tab.id)
                    && Self.isArchivable(tab.urlString)
                    && tab.lastAccessedAt < archiveCutoff
            }
            for tab in doomed { archiveTab(tab.id) }
        }

        if let sleepCutoff {
            let protected = sleepProtectedIDs
            for tab in tabs where !protected.contains(tab.id) {
                guard tab.hasRealized, !tab.isAsleep,
                      tab.lastAccessedAt < sleepCutoff else { continue }
                tab.sleep()
                didSleepAny = true
            }
        }

        // A slept tab leaves a now-defunct native subview parked in the web
        // container; nudge a re-render so it's detached and freed promptly.
        if didSleepAny { objectWillChange.send() }
    }

    // MARK: Manual actions (context menu / shortcuts)

    /// Put a single background tab to sleep now. No-op for the visible tabs.
    func sleepTab(_ id: BrowserTab.ID) {
        guard !sleepProtectedIDs.contains(id),
              let tab = tabs.first(where: { $0.id == id }),
              tab.hasRealized, !tab.isAsleep else { return }
        tab.sleep()
        objectWillChange.send()
        ToastCenter.shared.show("Tab put to sleep", icon: "moon.zzz", style: .success)
    }

    /// Sleep every eligible background tab right now (memory relief on demand).
    func sleepBackgroundTabs() {
        let protected = sleepProtectedIDs
        var count = 0
        for tab in tabs where !protected.contains(tab.id) {
            guard tab.hasRealized, !tab.isAsleep else { continue }
            tab.sleep()
            count += 1
        }
        guard count > 0 else {
            ToastCenter.shared.show("No background tabs to sleep", icon: "moon", style: .warning)
            return
        }
        objectWillChange.send()
        ToastCenter.shared.show("Slept \(count) tab\(count == 1 ? "" : "s")",
                                icon: "moon.zzz", style: .success)
    }

    /// Archive a tab: record it to the restorable Archive and close it.
    func archiveTab(_ id: BrowserTab.ID) {
        guard let tab = tabs.first(where: { $0.id == id }),
              !contexts.contains(where: { $0.pinnedTabIDs.contains(id) })
        else { return }
        ArchiveStore.shared.add(url: tab.urlString,
                                title: tab.title,
                                faviconURL: tab.faviconURL)
        closeTab(id, allowFolderRemoval: true)
    }

    /// Reopen an archived page in a fresh tab and drop it from the archive.
    @discardableResult
    func restoreArchived(_ archived: ArchivedTab) -> BrowserTab {
        let tab = newTab(url: archived.url, select: true)
        tab.faviconURL = archived.faviconURL
        ArchiveStore.shared.remove(archived)
        return tab
    }
}
