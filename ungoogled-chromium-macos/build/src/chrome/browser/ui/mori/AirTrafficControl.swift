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
