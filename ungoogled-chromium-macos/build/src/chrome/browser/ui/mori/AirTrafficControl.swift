import SwiftUI

/// Air Traffic Control: automatically move tabs into the space a routing rule
/// assigns to the host they land on, Arc-style.
extension BrowserStore {
    /// Evaluate routing for a freshly committed navigation.
    func applyRouting(for tab: BrowserTab, url: String) {
        guard let targetID = RouteStore.shared.matchingContextID(forURL: url),
              contexts.contains(where: { $0.id == targetID }),
              let currentIdx = contexts.firstIndex(where: { $0.tabIDs.contains(tab.id) }),
              contexts[currentIdx].id != targetID,
              !contexts[currentIdx].pinnedTabIDs.contains(tab.id)
        else { return }

        let follow = (tab.id == selectedTabID)
        moveTab(tab.id, toContext: targetID, activate: follow)
        let name = contexts.first { $0.id == targetID }?.name ?? "space"
        ToastCenter.shared.show("Routed to \(name)",
                                icon: "arrow.triangle.branch", style: .info)
    }

    /// Move a tab out of its current context and into `targetID`.
    func moveTab(_ id: BrowserTab.ID,
                 toContext targetID: BrowserContext.ID,
                 activate: Bool) {
        guard let targetIdx = contexts.firstIndex(where: { $0.id == targetID }) else { return }
        let wasSelected = (id == selectedTabID)

        for i in contexts.indices {
            contexts[i].tabIDs.removeAll { $0 == id }
            contexts[i].pinnedTabIDs.removeAll { $0 == id }
            for f in contexts[i].folders.indices {
                contexts[i].folders[f].tabIDs.removeAll { $0 == id }
            }
            if contexts[i].selectedTabID == id { contexts[i].selectedTabID = nil }
        }
        contexts[targetIdx].tabIDs.append(id)

        if activate {
            switchContext(to: targetID, selectRemembered: false)
            selectTab(id)
        } else if wasSelected {
            // The moved tab left the active context; keep a valid selection.
            if let fallback = activeContext.tabIDs.first { selectTab(fallback) }
        }
        scheduleSessionSave()
    }

    /// "Always Open in This Space": route the tab's host to the active context.
    func routeHostToActiveSpace(_ tabID: BrowserTab.ID) {
        guard let tab = tabs.first(where: { $0.id == tabID }) else { return }
        let host = RouteStore.normalize(tab.urlString)
        guard RouteStore.shared.add(pattern: host, contextID: activeContextID) else {
            ToastCenter.shared.show("Can't route this page", icon: "arrow.triangle.branch",
                                    style: .warning)
            return
        }
        ToastCenter.shared.show("\(host) → \(activeContext.name)",
                                icon: "arrow.triangle.branch", style: .success)
    }
}
