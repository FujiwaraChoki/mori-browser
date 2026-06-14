import Foundation

/// A named, collapsible group of tabs in the sidebar (Arc/SigmaOS-style folder).
/// Membership is stored as tab IDs; the store resolves them against live tabs so
/// a closed tab simply drops out of the folder.
struct TabFolder: Identifiable, Equatable, Codable {
    let id: UUID
    var name: String
    var symbol: String      // SF Symbol shown next to the name.
    var isExpanded: Bool
    var tabIDs: [UUID]

    init(id: UUID = UUID(),
         name: String,
         symbol: String = "folder",
         isExpanded: Bool = true,
         tabIDs: [UUID] = []) {
        self.id = id
        self.name = name
        self.symbol = symbol
        self.isExpanded = isExpanded
        self.tabIDs = tabIDs
    }
}
