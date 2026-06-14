import Foundation

/// One sidebar "context" — Mori's take on Arc's Spaces. Each context owns its
/// own sidebar organization (pinned tiles, folders, and the ordered set of
/// member tabs) plus an identity (name, glyph) and an optional chrome theme
/// that washes the window while the context is active.
///
/// Tabs themselves live in the store's flat pool; a context references them by
/// id, and every tab belongs to exactly one context. Stale ids are filtered on
/// resolve, so a closed tab simply drops out.
struct BrowserContext: Identifiable, Equatable, Codable {
    let id: UUID
    var name: String
    /// Glyph asset shown in the bottom-bar switcher (see `GlyphLibrary`).
    var symbol: String
    /// The chrome wash applied while this context is active. `.none` (empty)
    /// keeps the plain light/dark chrome.
    var theme: GradientTheme
    /// Member tabs in sidebar order. Pinned/foldered tabs keep their slot here
    /// so unpinning restores a sensible position.
    var tabIDs: [UUID]
    /// Tabs surfaced as icon tiles in the pinned grid, in order.
    var pinnedTabIDs: [UUID]
    /// Collapsible folders grouping member tabs.
    var folders: [TabFolder]
    /// The tab that was selected when this context was last active, restored
    /// on switch-back.
    var selectedTabID: UUID?

    init(id: UUID = UUID(),
         name: String,
         symbol: String = "glyph-circle",
         theme: GradientTheme = .none,
         tabIDs: [UUID] = [],
         pinnedTabIDs: [UUID] = [],
         folders: [TabFolder] = [],
         selectedTabID: UUID? = nil) {
        self.id = id
        self.name = name
        self.symbol = symbol
        self.theme = theme
        self.tabIDs = tabIDs
        self.pinnedTabIDs = pinnedTabIDs
        self.folders = folders
        self.selectedTabID = selectedTabID
    }
}
