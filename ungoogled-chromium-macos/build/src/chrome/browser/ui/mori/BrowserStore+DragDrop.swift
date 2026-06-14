import SwiftUI
import UniformTypeIdentifiers

enum SidebarTabDrag {
    static let acceptedTypes: [UTType] = [.plainText, .text]

    static func provider(for id: BrowserTab.ID) -> NSItemProvider {
        NSItemProvider(object: id.uuidString as NSString)
    }
}

/// Where a dragged sidebar tab should land. Indices are clamped by `moveTab`, so
/// a large value (e.g. `Int.max`) means "append".
enum TabDropTarget: Equatable {
    /// Insert into the pinned grid at this index.
    case pinned(index: Int)
    /// Insert into the given folder at this index (`Int.max` appends).
    case folder(id: TabFolder.ID, index: Int)
    /// Place at this position among the loose (unfiled) tabs.
    case loose(index: Int)
}

extension BrowserStore {
    /// Move a tab to a sidebar drop target, detaching it from whatever container
    /// it currently lives in first. Drives the live drag-and-drop reordering in
    /// the sidebar.
    ///
    /// Note: this intentionally does not emit `chrome.tabs.onMoved` extension
    /// events. Only the `.loose` branch touches the global `tabs` array, and
    /// sidebar organization (pin / folder membership / loose order) is a Mori
    /// concept that does not map cleanly onto the flat extension tab index.
    func moveTab(_ id: BrowserTab.ID, to target: TabDropTarget) {
        guard tabs.contains(where: { $0.id == id }) else { return }

        let sourcePinnedIndex = pinnedTabIDs.firstIndex(of: id)
        let sourceFolderIndex = folders.firstIndex { $0.tabIDs.contains(id) }
        let sourceFolderID = sourceFolderIndex.map { folders[$0].id }
        let sourceFolderTabIndex = sourceFolderIndex.flatMap { folders[$0].tabIDs.firstIndex(of: id) }
        let sourceLooseIndex = looseTabs.map(\.id).firstIndex(of: id)

        withAnimation(Motion.snappy) {
            // 1. Detach from the current container.
            pinnedTabIDs.removeAll { $0 == id }
            for i in folders.indices {
                folders[i].tabIDs.removeAll { $0 == id }
            }

            // 2. Apply the target.
            switch target {
            case .pinned(let index):
                let adjusted = adjustedInsertionIndex(index, movingFrom: sourcePinnedIndex)
                let clamped = min(max(adjusted, 0), pinnedTabIDs.count)
                pinnedTabIDs.insert(id, at: clamped)

            case .folder(let fid, let index):
                guard let fi = folders.firstIndex(where: { $0.id == fid }) else { return }
                let sourceIndex = sourceFolderID == fid ? sourceFolderTabIndex : nil
                let adjusted = adjustedInsertionIndex(index, movingFrom: sourceIndex)
                let clamped = min(max(adjusted, 0), folders[fi].tabIDs.count)
                folders[fi].tabIDs.insert(id, at: clamped)
                folders[fi].isExpanded = true

            case .loose(let index):
                // The tab is now loose (removed from pinned/folders above).
                // Rebuild the desired loose order, then weave it back into the
                // active context's member list, leaving pinned/foldered slots
                // untouched.
                var looseIDs = looseTabs.map(\.id).filter { $0 != id }
                let adjusted = adjustedInsertionIndex(index, movingFrom: sourceLooseIndex)
                let clamped = min(max(adjusted, 0), looseIDs.count)
                looseIDs.insert(id, at: clamped)

                var context = activeContext
                let looseSet = Set(looseIDs)
                var queue = looseIDs
                var rebuilt: [BrowserTab.ID] = []
                for memberID in context.tabIDs where memberID != id {
                    if looseSet.contains(memberID) {
                        if let nid = queue.first {
                            rebuilt.append(nid)
                            queue.removeFirst()
                        }
                    } else {
                        rebuilt.append(memberID)
                    }
                }
                // One desired entry is always left over (the walk skips the
                // dragged tab's old slot). Appending keeps the *loose* order —
                // the filtered sequence — exactly as desired.
                rebuilt.append(contentsOf: queue)
                context.tabIDs = rebuilt
                activeContext = context
            }
            syncChromePinnedState(for: id)
            scheduleSessionSave()
        }
    }
}

/// A reusable live-reordering drop delegate. As the dragged tab hovers over a
/// target element it is moved there immediately (animated), so the sidebar
/// rearranges under the cursor.
struct TabReorderDropDelegate: DropDelegate {
    /// Where the dragged tab should go if it is dropped on (or hovered over) THIS
    /// element.
    let target: TabDropTarget
    @Binding var draggingID: BrowserTab.ID?
    let store: BrowserStore
    /// Optional highlight flag for containers that want to show they're the
    /// active drop target (e.g. a folder row).
    var isTargeted: Binding<Bool>? = nil
    /// Large sidebar/chrome catch zones should accept a release without yanking
    /// the tab around while the user is only passing through them.
    var moveOnEnter = true

    func dropUpdated(info: DropInfo) -> DropProposal? {
        DropProposal(operation: .move)
    }

    func dropEntered(info: DropInfo) {
        isTargeted?.wrappedValue = true
        guard moveOnEnter else { return }
        resolveDraggedTabID(from: info) { id in
            move(id, clearingDragState: false)
        }
    }

    func dropExited(info: DropInfo) {
        isTargeted?.wrappedValue = false
    }

    func performDrop(info: DropInfo) -> Bool {
        // Commit the move here too: `dropEntered` gives live reordering while the
        // cursor moves, but it can miss (small targets, layout shifting under the
        // pointer as a folder expands). Releasing always lands the tab.
        resolveDraggedTabID(from: info) { id in
            move(id, clearingDragState: true)
        }
        return true
    }

    private func resolveDraggedTabID(from info: DropInfo,
                                     completion: @escaping (BrowserTab.ID?) -> Void) {
        if let draggingID {
            completion(draggingID)
            return
        }

        guard let provider = info.itemProviders(for: SidebarTabDrag.acceptedTypes).first else {
            completion(nil)
            return
        }

        provider.loadObject(ofClass: NSString.self) { object, _ in
            let id = (object as? String)
                .flatMap { BrowserTab.ID(uuidString: $0) }
            DispatchQueue.main.async {
                completion(id)
            }
        }
    }

    private func move(_ id: BrowserTab.ID?, clearingDragState: Bool) {
        defer {
            if clearingDragState { draggingID = nil }
            if clearingDragState { isTargeted?.wrappedValue = false }
        }

        guard let id else { return }
        if !store.isAlready(id, at: target) {
            store.moveTab(id, to: target)
        }
    }
}

extension BrowserStore {
    /// True if `id` already occupies `target`, so a hover move would be a no-op
    /// (avoids churn / flicker during live reordering).
    func isAlready(_ id: BrowserTab.ID, at target: TabDropTarget) -> Bool {
        switch target {
        case .pinned(let index):
            guard let current = pinnedTabIDs.firstIndex(of: id) else { return false }
            let adjusted = adjustedInsertionIndex(index, movingFrom: current)
            return current == clampedIndex(adjusted, count: max(pinnedTabIDs.count - 1, 0))
        case .folder(let fid, let index):
            guard let folder = folders.first(where: { $0.id == fid }) else { return false }
            guard let current = folder.tabIDs.firstIndex(of: id) else { return false }
            let adjusted = adjustedInsertionIndex(index, movingFrom: current)
            return current == clampedIndex(adjusted, count: max(folder.tabIDs.count - 1, 0))
        case .loose(let index):
            let ids = looseTabs.map(\.id)
            guard let current = ids.firstIndex(of: id) else { return false }
            let adjusted = adjustedInsertionIndex(index, movingFrom: current)
            return current == clampedIndex(adjusted, count: max(ids.count - 1, 0))
        }
    }

    private func clampedIndex(_ index: Int, count: Int) -> Int {
        min(max(index, 0), count)
    }

    private func adjustedInsertionIndex(_ index: Int, movingFrom sourceIndex: Int?) -> Int {
        guard let sourceIndex, sourceIndex < index else { return index }
        return index - 1
    }
}
