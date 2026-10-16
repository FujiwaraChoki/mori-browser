//! Tabs, spaces, and folders.
//!
//! A [`Workspace`] is the persisted shape of the sidebar: every tab belongs
//! to exactly one [`Space`], and inside a space it is either pinned, inside a
//! [`Folder`], or loose. Live browser state (loading, page views) lives in
//! the UI; this module only owns membership, order, and selection, so every
//! rule here is unit-testable.

use std::{collections::HashMap, path::Path};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::theme::GradientTheme;

pub type TabId = Uuid;
pub type SpaceId = Uuid;
pub type FolderId = Uuid;

/// The persisted part of a tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabRecord {
    pub id: TabId,
    pub url: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub favicon_url: Option<String>,
    #[serde(default)]
    pub custom_title: Option<String>,
    /// UNIX seconds; drives sleep/archive and launcher recency.
    #[serde(default)]
    pub last_accessed: f64,
}

impl TabRecord {
    pub fn new(url: impl Into<String>) -> Self {
        TabRecord {
            id: Uuid::new_v4(),
            url: url.into(),
            title: String::new(),
            favicon_url: None,
            custom_title: None,
            last_accessed: crate::paths::now(),
        }
    }

    /// Custom title if set, else the page title, else "Untitled".
    pub fn display_title(&self) -> &str {
        if let Some(t) = self.custom_title.as_deref()
            && !t.trim().is_empty()
        {
            return t;
        }
        if self.title.trim().is_empty() {
            if self.url.is_empty() || self.url == "about:blank" || self.url.starts_with("mori://") {
                "New Tab"
            } else {
                "Untitled"
            }
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: FolderId,
    pub name: String,
    #[serde(default = "default_folder_symbol")]
    pub symbol: String,
    #[serde(default = "yes")]
    pub is_expanded: bool,
    #[serde(default)]
    pub tab_ids: Vec<TabId>,
}

fn default_folder_symbol() -> String {
    "folder".into()
}

fn default_space_symbol() -> String {
    "circle".into()
}

/// Symbols (Lucide names) handed to new spaces in turn, so every space is
/// recognisable in the switcher. The first space keeps the plain default.
pub const NEW_SPACE_SYMBOLS: [&str; 10] = [
    "briefcase", "code", "book-open", "palette", "music", "leaf", "star", "heart", "flame", "globe",
];

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    #[serde(default = "default_space_symbol")]
    pub symbol: String,
    #[serde(default = "GradientTheme::none")]
    pub theme: GradientTheme,
    #[serde(default)]
    pub pinned: Vec<TabId>,
    #[serde(default)]
    pub folders: Vec<Folder>,
    /// Tabs that are neither pinned nor in a folder, in sidebar order.
    #[serde(default)]
    pub loose: Vec<TabId>,
    #[serde(default)]
    pub selected: Option<TabId>,
}

impl Space {
    pub fn new(name: impl Into<String>) -> Self {
        Space {
            id: Uuid::new_v4(),
            name: name.into(),
            symbol: default_space_symbol(),
            theme: GradientTheme::none(),
            pinned: Vec::new(),
            folders: Vec::new(),
            loose: Vec::new(),
            selected: None,
        }
    }

    pub fn contains(&self, tab: TabId) -> bool {
        self.pinned.contains(&tab)
            || self.loose.contains(&tab)
            || self.folders.iter().any(|f| f.tab_ids.contains(&tab))
    }

    /// Every tab in sidebar order: pinned, then folders, then loose.
    pub fn ordered(&self) -> Vec<TabId> {
        let mut out = self.pinned.clone();
        for f in &self.folders {
            out.extend(&f.tab_ids);
        }
        out.extend(&self.loose);
        out
    }

    /// Tabs currently visible in the sidebar (collapsed folders hide theirs).
    pub fn visible(&self) -> Vec<TabId> {
        let mut out = self.pinned.clone();
        for f in self.folders.iter().filter(|f| f.is_expanded) {
            out.extend(&f.tab_ids);
        }
        out.extend(&self.loose);
        out
    }

    pub fn folder(&self, id: FolderId) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    fn folder_mut(&mut self, id: FolderId) -> Option<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    fn slot_of(&self, tab: TabId) -> Option<Slot> {
        if let Some(i) = self.pinned.iter().position(|t| *t == tab) {
            return Some(Slot::Pinned(i));
        }
        for f in &self.folders {
            if let Some(i) = f.tab_ids.iter().position(|t| *t == tab) {
                return Some(Slot::Folder(f.id, i));
            }
        }
        self.loose.iter().position(|t| *t == tab).map(Slot::Loose)
    }

    /// Remove `tab` from whichever list holds it.
    fn detach(&mut self, tab: TabId) -> Option<Slot> {
        let slot = self.slot_of(tab)?;
        match slot {
            Slot::Pinned(i) => {
                self.pinned.remove(i);
            }
            Slot::Folder(fid, i) => {
                if let Some(f) = self.folder_mut(fid) {
                    f.tab_ids.remove(i);
                }
            }
            Slot::Loose(i) => {
                self.loose.remove(i);
            }
        }
        Some(slot)
    }

    /// Insert at `slot`, clamping indices. Returns false for a missing folder.
    fn attach(&mut self, tab: TabId, slot: Slot) -> bool {
        match slot {
            Slot::Pinned(i) => {
                let i = i.min(self.pinned.len());
                self.pinned.insert(i, tab);
            }
            Slot::Folder(fid, i) => {
                let Some(f) = self.folder_mut(fid) else {
                    return false;
                };
                let i = i.min(f.tab_ids.len());
                f.tab_ids.insert(i, tab);
                f.is_expanded = true;
            }
            Slot::Loose(i) => {
                let i = i.min(self.loose.len());
                self.loose.insert(i, tab);
            }
        }
        true
    }
}

/// A position inside one space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Pinned(usize),
    Folder(FolderId, usize),
    Loose(usize),
}

impl Slot {
    pub fn is_pinned(&self) -> bool {
        matches!(self, Slot::Pinned(_))
    }

    pub fn folder(&self) -> Option<FolderId> {
        match self {
            Slot::Folder(f, _) => Some(*f),
            _ => None,
        }
    }

    fn same_list(&self, other: &Slot) -> bool {
        match (self, other) {
            (Slot::Pinned(_), Slot::Pinned(_)) | (Slot::Loose(_), Slot::Loose(_)) => true,
            (Slot::Folder(a, _), Slot::Folder(b, _)) => a == b,
            _ => false,
        }
    }

    fn index(&self) -> usize {
        match self {
            Slot::Pinned(i) | Slot::Folder(_, i) | Slot::Loose(i) => *i,
        }
    }

    fn with_index(&self, i: usize) -> Slot {
        match self {
            Slot::Pinned(_) => Slot::Pinned(i),
            Slot::Folder(f, _) => Slot::Folder(*f, i),
            Slot::Loose(_) => Slot::Loose(i),
        }
    }
}

/// Where a tab lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Location {
    pub space: SpaceId,
    pub slot: Slot,
}

/// A closed tab and where it was, for "Undo" and reopen-closed-tab.
#[derive(Clone, Debug)]
pub struct ClosedTab {
    pub record: TabRecord,
    pub location: Location,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default = "session_version")]
    pub version: u32,
    pub tabs: Vec<TabRecord>,
    pub spaces: Vec<Space>,
    pub active_space: SpaceId,
    /// Not persisted: lookup from id to index in `tabs`.
    #[serde(skip)]
    index: HashMap<TabId, usize>,
}

fn session_version() -> u32 {
    3
}

pub const SESSION_FILE: &str = "session.json";

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    /// A workspace with one empty "Personal" space.
    pub fn new() -> Self {
        let space = Space::new("Personal");
        Workspace {
            version: session_version(),
            active_space: space.id,
            tabs: Vec::new(),
            spaces: vec![space],
            index: HashMap::new(),
        }
    }

    pub fn load(dir: &Path) -> Workspace {
        let mut ws: Workspace =
            crate::paths::read_json(&dir.join(SESSION_FILE)).unwrap_or_default();
        ws.repair();
        ws
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        crate::paths::write_json_atomic(&dir.join(SESSION_FILE), self)
    }

    /// Restore invariants after loading untrusted data: drop dangling ids and
    /// duplicates, adopt orphan tabs, and keep at least one space.
    pub fn repair(&mut self) {
        if self.spaces.is_empty() {
            self.spaces.push(Space::new("Personal"));
        }
        let known: std::collections::HashSet<TabId> = self.tabs.iter().map(|t| t.id).collect();
        let mut seen = std::collections::HashSet::new();
        for space in &mut self.spaces {
            let mut keep = |id: &TabId| known.contains(id) && seen.insert(*id);
            space.pinned.retain(&mut keep);
            for f in &mut space.folders {
                f.tab_ids.retain(&mut keep);
            }
            space.loose.retain(&mut keep);
            if space.selected.is_some_and(|s| !space.contains(s)) {
                space.selected = None;
            }
        }
        if !self.spaces.iter().any(|s| s.id == self.active_space) {
            self.active_space = self.spaces[0].id;
        }
        let orphans: Vec<TabId> = self
            .tabs
            .iter()
            .map(|t| t.id)
            .filter(|id| !seen.contains(id))
            .collect();
        let active = self.active_space;
        if let Some(space) = self.space_mut(active) {
            space.loose.extend(orphans);
        }
        self.reindex();
    }

    fn reindex(&mut self) {
        self.index = self
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id, i))
            .collect();
    }

    // ---- Lookup -------------------------------------------------------------

    pub fn tab(&self, id: TabId) -> Option<&TabRecord> {
        self.index.get(&id).map(|&i| &self.tabs[i])
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut TabRecord> {
        self.index.get(&id).map(|&i| &mut self.tabs[i])
    }

    pub fn space(&self, id: SpaceId) -> Option<&Space> {
        self.spaces.iter().find(|s| s.id == id)
    }

    pub fn space_mut(&mut self, id: SpaceId) -> Option<&mut Space> {
        self.spaces.iter_mut().find(|s| s.id == id)
    }

    pub fn active(&self) -> &Space {
        self.space(self.active_space).unwrap_or(&self.spaces[0])
    }

    pub fn active_mut(&mut self) -> &mut Space {
        let id = self.active_space;
        let i = self.spaces.iter().position(|s| s.id == id).unwrap_or(0);
        &mut self.spaces[i]
    }

    pub fn selected(&self) -> Option<TabId> {
        self.active().selected
    }

    pub fn location(&self, tab: TabId) -> Option<Location> {
        self.spaces.iter().find_map(|s| {
            s.slot_of(tab).map(|slot| Location {
                space: s.id,
                slot,
            })
        })
    }

    pub fn space_of(&self, tab: TabId) -> Option<SpaceId> {
        self.location(tab).map(|l| l.space)
    }

    pub fn is_pinned(&self, tab: TabId) -> bool {
        self.location(tab).is_some_and(|l| l.slot.is_pinned())
    }

    pub fn folder_of(&self, tab: TabId) -> Option<FolderId> {
        self.location(tab).and_then(|l| l.slot.folder())
    }

    pub fn find_folder(&self, folder: FolderId) -> Option<(SpaceId, &Folder)> {
        self.spaces
            .iter()
            .find_map(|s| s.folder(folder).map(|f| (s.id, f)))
    }

    // ---- Tabs ---------------------------------------------------------------

    /// Add a new tab at `slot` in `space` (the active space if `None`).
    pub fn add_tab(&mut self, record: TabRecord, space: Option<SpaceId>, slot: Slot) -> TabId {
        let id = record.id;
        let space_id = space.unwrap_or(self.active_space);
        self.tabs.push(record);
        self.index.insert(id, self.tabs.len() - 1);
        let space = match self.space_mut(space_id) {
            Some(s) => s,
            None => self.active_mut(),
        };
        if !space.attach(id, slot) {
            space.loose.push(id);
        }
        id
    }

    /// Add a tab directly after `after` in the same list (loose end if unknown).
    pub fn add_tab_after(&mut self, record: TabRecord, after: Option<TabId>) -> TabId {
        match after.and_then(|a| self.location(a)) {
            Some(loc) if !loc.slot.is_pinned() => {
                let slot = loc.slot.with_index(loc.slot.index() + 1);
                self.add_tab(record, Some(loc.space), slot)
            }
            Some(loc) => self.add_tab(record, Some(loc.space), Slot::Loose(0)),
            None => {
                let end = self.active().loose.len();
                self.add_tab(record, None, Slot::Loose(end))
            }
        }
    }

    /// Remove a tab, returning it with its location for undo. The space's
    /// selection moves to a neighbour if it was selected.
    pub fn remove_tab(&mut self, id: TabId) -> Option<ClosedTab> {
        let location = self.location(id)?;
        let neighbor = self.neighbor(id);
        let space = self.space_mut(location.space)?;
        space.detach(id);
        if space.selected == Some(id) {
            space.selected = neighbor;
        }
        let i = *self.index.get(&id)?;
        let record = self.tabs.remove(i);
        self.reindex();
        Some(ClosedTab { record, location })
    }

    /// Put a closed tab back where it was (as close as still possible).
    pub fn restore_tab(&mut self, closed: ClosedTab) -> TabId {
        let space = if self.space(closed.location.space).is_some() {
            Some(closed.location.space)
        } else {
            None
        };
        let slot = match closed.location.slot {
            Slot::Folder(f, i) if space.and_then(|s| self.space(s)?.folder(f)).is_none() => {
                Slot::Loose(i)
            }
            other => other,
        };
        self.add_tab(closed.record, space, slot)
    }

    /// The tab that should become selected if `id` goes away: the next tab in
    /// sidebar order, else the previous one.
    pub fn neighbor(&self, id: TabId) -> Option<TabId> {
        let space = self.space(self.space_of(id)?)?;
        let order = space.visible();
        let i = order.iter().position(|t| *t == id)?;
        order
            .get(i + 1)
            .or_else(|| i.checked_sub(1).and_then(|j| order.get(j)))
            .copied()
    }

    /// Select a tab, switching to its space. Returns false if unknown.
    pub fn select(&mut self, id: TabId) -> bool {
        let Some(space) = self.space_of(id) else {
            return false;
        };
        self.active_space = space;
        if let Some(t) = self.tab_mut(id) {
            t.last_accessed = crate::paths::now();
        }
        self.active_mut().selected = Some(id);
        true
    }

    /// Move a tab to `slot` in `space`. Moving down within the same list
    /// accounts for the tab's own removal, so "drop before index i" works.
    pub fn move_tab(&mut self, id: TabId, space: SpaceId, slot: Slot) -> bool {
        let Some(from) = self.location(id) else {
            return false;
        };
        if self.space(space).is_none() {
            return false;
        }
        if let Slot::Folder(f, _) = slot
            && self.space(space).and_then(|s| s.folder(f)).is_none()
        {
            return false;
        }
        let mut target = slot;
        if from.space == space && from.slot.same_list(&slot) {
            let (fi, ti) = (from.slot.index(), slot.index());
            if fi == ti || fi + 1 == ti {
                return true; // already there
            }
            if fi < ti {
                target = slot.with_index(ti - 1);
            }
        }
        let was_selected = self.space(from.space).and_then(|s| s.selected) == Some(id);
        let neighbor = if from.space != space && was_selected {
            self.neighbor(id)
        } else {
            None
        };
        if let Some(s) = self.space_mut(from.space) {
            s.detach(id);
            if from.space != space && was_selected {
                s.selected = neighbor;
            }
        }
        if let Some(s) = self.space_mut(space) {
            s.attach(id, target);
        }
        true
    }

    /// Move a tab to the end of another space's loose list.
    pub fn move_to_space(&mut self, id: TabId, space: SpaceId) -> bool {
        let end = self.space(space).map(|s| s.loose.len()).unwrap_or(0);
        self.move_tab(id, space, Slot::Loose(end))
    }

    pub fn pin(&mut self, id: TabId) -> bool {
        let Some(loc) = self.location(id) else {
            return false;
        };
        if loc.slot.is_pinned() {
            return true;
        }
        let end = self.space(loc.space).map(|s| s.pinned.len()).unwrap_or(0);
        self.move_tab(id, loc.space, Slot::Pinned(end))
    }

    pub fn unpin(&mut self, id: TabId) -> bool {
        let Some(loc) = self.location(id) else {
            return false;
        };
        if !loc.slot.is_pinned() {
            return true;
        }
        self.move_tab(id, loc.space, Slot::Loose(0))
    }

    pub fn toggle_pin(&mut self, id: TabId) -> bool {
        if self.is_pinned(id) {
            self.unpin(id);
            false
        } else {
            self.pin(id);
            true
        }
    }

    /// Ordered tabs of the active space (pinned, folders, loose).
    pub fn ordered(&self) -> Vec<TabId> {
        self.active().ordered()
    }

    /// The tab `delta` steps from the selection, wrapping, in the active space.
    pub fn cycle(&self, delta: isize) -> Option<TabId> {
        let order = self.active().visible();
        if order.is_empty() {
            return None;
        }
        let n = order.len() as isize;
        let current = self
            .selected()
            .and_then(|s| order.iter().position(|t| *t == s))
            .map(|i| i as isize)
            .unwrap_or(-1);
        let next = (current + delta).rem_euclid(n);
        order.get(next as usize).copied()
    }

    /// Ctrl+1..9: 1-8 pick by position, 9 picks the last tab.
    pub fn by_ordinal(&self, n: usize) -> Option<TabId> {
        let order = self.ordered();
        if n == 9 {
            return order.last().copied();
        }
        order.get(n.checked_sub(1)?).copied()
    }

    // ---- Folders --------------------------------------------------------------

    /// Create a folder in `space` holding `tabs` (which leave their old slots).
    pub fn create_folder(
        &mut self,
        space: SpaceId,
        name: impl Into<String>,
        tabs: &[TabId],
    ) -> Option<FolderId> {
        let folder = Folder {
            id: Uuid::new_v4(),
            name: name.into(),
            symbol: default_folder_symbol(),
            is_expanded: true,
            tab_ids: Vec::new(),
        };
        let fid = folder.id;
        self.space_mut(space)?.folders.push(folder);
        for (i, tab) in tabs.iter().enumerate() {
            self.move_tab(*tab, space, Slot::Folder(fid, i));
        }
        Some(fid)
    }

    /// Delete a folder; its tabs become loose tabs at the top of the list.
    pub fn delete_folder(&mut self, folder: FolderId) -> bool {
        let Some((space_id, _)) = self.find_folder(folder) else {
            return false;
        };
        let Some(space) = self.space_mut(space_id) else {
            return false;
        };
        let Some(i) = space.folders.iter().position(|f| f.id == folder) else {
            return false;
        };
        let removed = space.folders.remove(i);
        for (j, tab) in removed.tab_ids.into_iter().enumerate() {
            space.loose.insert(j, tab);
        }
        true
    }

    pub fn update_folder(&mut self, folder: FolderId, f: impl FnOnce(&mut Folder)) -> bool {
        for space in &mut self.spaces {
            if let Some(found) = space.folder_mut(folder) {
                f(found);
                return true;
            }
        }
        false
    }

    // ---- Spaces ---------------------------------------------------------------

    pub fn create_space(&mut self, name: impl Into<String>) -> SpaceId {
        let mut space = Space::new(name);
        space.symbol = self.next_space_symbol().to_string();
        let id = space.id;
        self.spaces.push(space);
        id
    }

    /// The first symbol in [`NEW_SPACE_SYMBOLS`] no space uses yet; once all
    /// are taken they repeat in order.
    pub fn next_space_symbol(&self) -> &'static str {
        NEW_SPACE_SYMBOLS
            .iter()
            .find(|sym| !self.spaces.iter().any(|s| s.symbol == **sym))
            .copied()
            .unwrap_or(NEW_SPACE_SYMBOLS[self.spaces.len() % NEW_SPACE_SYMBOLS.len()])
    }

    /// Default name for the next new space: "Space N".
    pub fn next_space_name(&self) -> String {
        format!("Space {}", self.spaces.len() + 1)
    }

    /// Delete a space and return its tabs (now closed). Refuses to delete the
    /// last space. Falls back to the previous space if it was active.
    pub fn delete_space(&mut self, id: SpaceId) -> Option<Vec<TabRecord>> {
        if self.spaces.len() <= 1 {
            return None;
        }
        let i = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(i);
        let doomed: std::collections::HashSet<TabId> = space.ordered().into_iter().collect();
        let (closed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tabs)
            .into_iter()
            .partition(|t| doomed.contains(&t.id));
        self.tabs = kept;
        self.reindex();
        if self.active_space == id {
            self.active_space = self.spaces[i.saturating_sub(1).min(self.spaces.len() - 1)].id;
        }
        Some(closed)
    }

    pub fn switch_space(&mut self, id: SpaceId) -> bool {
        if self.space(id).is_some() {
            self.active_space = id;
            true
        } else {
            false
        }
    }

    /// Space `n` (1-based) in switcher order.
    pub fn space_by_ordinal(&self, n: usize) -> Option<SpaceId> {
        self.spaces.get(n.checked_sub(1)?).map(|s| s.id)
    }

    pub fn tab_count(&self, space: SpaceId) -> usize {
        self.space(space).map(|s| s.ordered().len()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws_with(n: usize) -> (Workspace, Vec<TabId>) {
        let mut ws = Workspace::new();
        let ids = (0..n)
            .map(|i| {
                let end = ws.active().loose.len();
                ws.add_tab(TabRecord::new(format!("https://t{i}.test")), None, Slot::Loose(end))
            })
            .collect();
        (ws, ids)
    }

    #[test]
    fn add_and_order() {
        let (mut ws, ids) = ws_with(3);
        assert_eq!(ws.ordered(), ids);
        ws.pin(ids[2]);
        assert_eq!(ws.ordered(), vec![ids[2], ids[0], ids[1]]);
        assert!(ws.is_pinned(ids[2]));
    }

    #[test]
    fn remove_selects_neighbor_and_restore() {
        let (mut ws, ids) = ws_with(3);
        ws.select(ids[1]);
        let closed = ws.remove_tab(ids[1]).unwrap();
        assert_eq!(ws.selected(), Some(ids[2]));
        assert_eq!(ws.ordered(), vec![ids[0], ids[2]]);
        ws.restore_tab(closed);
        assert_eq!(ws.ordered(), ids);
    }

    #[test]
    fn last_tab_neighbor_is_previous() {
        let (mut ws, ids) = ws_with(2);
        ws.select(ids[1]);
        ws.remove_tab(ids[1]);
        assert_eq!(ws.selected(), Some(ids[0]));
    }

    #[test]
    fn move_down_within_list_adjusts_index() {
        let (mut ws, ids) = ws_with(4);
        let space = ws.active_space;
        // Drop tab 0 before index 3 (between t2 and t3).
        ws.move_tab(ids[0], space, Slot::Loose(3));
        assert_eq!(ws.ordered(), vec![ids[1], ids[2], ids[0], ids[3]]);
        // Moving onto its own position is a no-op.
        ws.move_tab(ids[0], space, Slot::Loose(2));
        assert_eq!(ws.ordered(), vec![ids[1], ids[2], ids[0], ids[3]]);
        // Move up.
        ws.move_tab(ids[3], space, Slot::Loose(0));
        assert_eq!(ws.ordered(), vec![ids[3], ids[1], ids[2], ids[0]]);
    }

    #[test]
    fn folders() {
        let (mut ws, ids) = ws_with(3);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[0], ids[2]]).unwrap();
        assert_eq!(ws.folder_of(ids[0]), Some(f));
        assert_eq!(ws.active().loose, vec![ids[1]]);
        assert_eq!(ws.ordered(), vec![ids[0], ids[2], ids[1]]);
        ws.update_folder(f, |f| f.is_expanded = false);
        assert_eq!(ws.active().visible(), vec![ids[1]]);
        assert!(ws.delete_folder(f));
        assert_eq!(ws.active().loose, vec![ids[0], ids[2], ids[1]]);
    }

    #[test]
    fn pinning_leaves_folder() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let f = ws.create_folder(space, "F", &[ids[0]]).unwrap();
        ws.pin(ids[0]);
        assert!(ws.active().folder(f).unwrap().tab_ids.is_empty());
        assert!(ws.is_pinned(ids[0]));
    }

    #[test]
    fn spaces_switch_and_delete() {
        let (mut ws, ids) = ws_with(2);
        let personal = ws.active_space;
        let work = ws.create_space("Work");
        ws.move_to_space(ids[1], work);
        assert_eq!(ws.space_of(ids[1]), Some(work));
        ws.select(ids[1]);
        assert_eq!(ws.active_space, work);
        let closed = ws.delete_space(work).unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(ws.active_space, personal);
        assert!(ws.tab(ids[1]).is_none());
        assert!(ws.delete_space(personal).is_none(), "last space is kept");
    }

    #[test]
    fn new_spaces_get_distinct_symbols() {
        let (mut ws, _) = ws_with(0);
        let first = ws.active().symbol.clone();
        let a = ws.create_space("A");
        let b = ws.create_space("B");
        let symbols: Vec<&str> = [ws.active_space, a, b]
            .iter()
            .map(|id| ws.space(*id).unwrap().symbol.as_str())
            .collect();
        assert_eq!(symbols, [first.as_str(), NEW_SPACE_SYMBOLS[0], NEW_SPACE_SYMBOLS[1]]);
        // A symbol freed by deletion is handed out again before new ones.
        ws.delete_space(a);
        let c = ws.create_space("C");
        assert_eq!(ws.space(c).unwrap().symbol, NEW_SPACE_SYMBOLS[0]);
        // Past the list, symbols repeat instead of failing.
        for i in 0..NEW_SPACE_SYMBOLS.len() {
            ws.create_space(format!("S{i}"));
        }
        assert!(NEW_SPACE_SYMBOLS.contains(&ws.spaces.last().unwrap().symbol.as_str()));
    }

    #[test]
    fn cycle_and_ordinal() {
        let (mut ws, ids) = ws_with(3);
        ws.select(ids[2]);
        assert_eq!(ws.cycle(1), Some(ids[0]));
        assert_eq!(ws.cycle(-1), Some(ids[1]));
        assert_eq!(ws.by_ordinal(1), Some(ids[0]));
        assert_eq!(ws.by_ordinal(9), Some(ids[2]));
        assert_eq!(ws.by_ordinal(5), None);
    }

    #[test]
    fn repair_drops_dangling_and_adopts_orphans() {
        let (mut ws, ids) = ws_with(2);
        ws.active_mut().loose.retain(|t| *t != ids[1]);
        ws.active_mut().pinned.push(Uuid::new_v4());
        ws.active_mut().loose.push(ids[0]); // duplicate
        ws.repair();
        assert_eq!(ws.ordered(), vec![ids[0], ids[1]]);
    }

    #[test]
    fn serde_roundtrip() {
        let (mut ws, ids) = ws_with(2);
        ws.select(ids[0]);
        let json = serde_json::to_string(&ws).unwrap();
        let mut back: Workspace = serde_json::from_str(&json).unwrap();
        back.repair();
        assert_eq!(back.ordered(), ids);
        assert_eq!(back.tab(ids[1]).unwrap().url, "https://t1.test");
        assert_eq!(back.selected(), Some(ids[0]));
    }
}
