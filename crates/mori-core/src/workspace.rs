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
    /// Chromium zoom level (0 = 100%), restored when the page loads again.
    #[serde(default)]
    pub zoom_level: f64,
    /// Last known vertical scroll offset in CSS pixels, restored when a
    /// slept or restored tab loads again.
    #[serde(default)]
    pub scroll_y: f64,
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
            zoom_level: 0.0,
            scroll_y: 0.0,
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

fn yes() -> bool {
    true
}

/// Narrowest share of the window either side of a split may take.
pub const SPLIT_MIN_RATIO: f32 = 0.2;

/// Two tabs shown side by side. `tabs[0]` is the left pane. The pair is
/// remembered, so selecting either tab shows both again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SplitGroup {
    pub tabs: [TabId; 2],
    /// Width share of the left pane.
    #[serde(default = "half")]
    pub ratio: f32,
}

fn half() -> f32 {
    0.5
}

impl SplitGroup {
    pub fn new(left: TabId, right: TabId) -> Self {
        SplitGroup {
            tabs: [left, right],
            ratio: 0.5,
        }
    }

    pub fn contains(&self, tab: TabId) -> bool {
        self.tabs.contains(&tab)
    }

    /// The other tab of the pair.
    pub fn partner(&self, tab: TabId) -> Option<TabId> {
        match self.tabs {
            [a, b] if a == tab => Some(b),
            [a, b] if b == tab => Some(a),
            _ => None,
        }
    }

    /// The ratio clamped so neither pane collapses (NaN → even split).
    pub fn clamped_ratio(&self) -> f32 {
        if self.ratio.is_nan() {
            return 0.5;
        }
        self.ratio.clamp(SPLIT_MIN_RATIO, 1.0 - SPLIT_MIN_RATIO)
    }
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
    /// Remembered split-view pairs.
    #[serde(default)]
    pub splits: Vec<SplitGroup>,
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
            splits: Vec::new(),
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

    /// The split pair `tab` belongs to, if any.
    pub fn split_of(&self, tab: TabId) -> Option<&SplitGroup> {
        self.splits.iter().find(|g| g.contains(tab))
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

    /// Whether both slots are in the same list (pinned, one folder, or loose).
    pub fn same_list(&self, other: &Slot) -> bool {
        match (self, other) {
            (Slot::Pinned(_), Slot::Pinned(_)) | (Slot::Loose(_), Slot::Loose(_)) => true,
            (Slot::Folder(a, _), Slot::Folder(b, _)) => a == b,
            _ => false,
        }
    }

    /// Position within its list.
    pub fn index(&self) -> usize {
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub space: SpaceId,
    pub slot: Slot,
}

/// A closed tab and where it was, for "Undo" and reopen-closed-tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClosedTab {
    pub record: TabRecord,
    pub location: Location,
}

/// How many closed tabs reopen-closed-tab remembers.
pub const MAX_CLOSED_TABS: usize = 25;

/// A deleted space with everything needed to put it back.
#[derive(Clone, Debug, PartialEq)]
pub struct DeletedSpace {
    pub space: Space,
    /// Its position in the switcher.
    pub index: usize,
    pub tabs: Vec<TabRecord>,
    /// Whether it was the active space.
    pub was_active: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default = "session_version")]
    pub version: u32,
    pub tabs: Vec<TabRecord>,
    pub spaces: Vec<Space>,
    pub active_space: SpaceId,
    /// Recently closed tabs, oldest first, for reopen-closed-tab.
    #[serde(default)]
    pub closed: Vec<ClosedTab>,
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
            closed: Vec::new(),
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

    /// Restore invariants after loading untrusted data: drop duplicate tab
    /// records, dangling ids and duplicate memberships, adopt orphan tabs,
    /// drop broken split pairs, and keep at least one space.
    pub fn repair(&mut self) {
        if self.spaces.is_empty() {
            self.spaces.push(Space::new("Personal"));
        }
        let mut known = std::collections::HashSet::new();
        self.tabs.retain(|t| known.insert(t.id));
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
        for space in &mut self.spaces {
            let mut paired = std::collections::HashSet::new();
            let members: std::collections::HashSet<TabId> = space.ordered().into_iter().collect();
            space.splits.retain(|g| {
                let [a, b] = g.tabs;
                let valid = a != b
                    && members.contains(&a)
                    && members.contains(&b)
                    && !paired.contains(&a)
                    && !paired.contains(&b);
                if valid {
                    paired.insert(a);
                    paired.insert(b);
                }
                valid
            });
        }
        self.closed.retain(|c| !known.contains(&c.record.id));
        let overflow = self.closed.len().saturating_sub(MAX_CLOSED_TABS);
        self.closed.drain(..overflow);
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
    /// A record whose id is already taken gets a fresh id, so two rows can
    /// never share one; the id actually used is returned.
    pub fn add_tab(&mut self, mut record: TabRecord, space: Option<SpaceId>, slot: Slot) -> TabId {
        if self.index.contains_key(&record.id) {
            record.id = Uuid::new_v4();
        }
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

    /// Add a tab the user opened (new tab, typed address, external link):
    /// like Arc, it goes to the top of the active space's loose list, never
    /// into a folder or the pinned grid.
    pub fn add_new_tab(&mut self, record: TabRecord) -> TabId {
        self.add_tab(record, None, Slot::Loose(0))
    }

    /// Add a tab opened from `opener` (a link, a duplicate, a promoted peek).
    ///
    /// It goes right below the opener when the opener is loose; tabs opened
    /// from pinned or foldered tabs go to the top of that space's loose list,
    /// because folders and pins are saved collections. `last_child` is the
    /// tab previously opened from the same opener (while the user stays on
    /// it): the new tab goes after that one, so repeated middle-clicks keep
    /// their order.
    pub fn add_child_tab(&mut self, record: TabRecord, opener: TabId, last_child: Option<TabId>) -> TabId {
        let Some(origin) = self.location(opener) else {
            return self.add_new_tab(record);
        };
        let anchor = last_child
            .and_then(|c| self.location(c))
            .filter(|l| l.space == origin.space && matches!(l.slot, Slot::Loose(_)))
            .or_else(|| matches!(origin.slot, Slot::Loose(_)).then_some(origin));
        match anchor {
            Some(loc) => self.add_tab(record, Some(loc.space), Slot::Loose(loc.slot.index() + 1)),
            None => self.add_tab(record, Some(origin.space), Slot::Loose(0)),
        }
    }

    /// Remove a tab, returning it with its location for undo. The space's
    /// selection moves to a neighbour if it was selected, and a split pair
    /// it belonged to is dissolved.
    pub fn remove_tab(&mut self, id: TabId) -> Option<ClosedTab> {
        let location = self.location(id)?;
        let neighbor = self.neighbor(id);
        let space = self.space_mut(location.space)?;
        space.detach(id);
        space.splits.retain(|g| !g.contains(id));
        if space.selected == Some(id) {
            space.selected = neighbor;
        }
        let i = *self.index.get(&id)?;
        let record = self.tabs.remove(i);
        self.reindex();
        Some(ClosedTab { record, location })
    }

    /// Close a tab the user can reopen later: remove it and remember it on
    /// the recently-closed stack.
    pub fn close_tab(&mut self, id: TabId) -> Option<ClosedTab> {
        let closed = self.remove_tab(id)?;
        self.closed.push(closed.clone());
        let overflow = self.closed.len().saturating_sub(MAX_CLOSED_TABS);
        self.closed.drain(..overflow);
        Some(closed)
    }

    /// Reopen the most recently closed tab that isn't open already.
    pub fn reopen_closed(&mut self) -> Option<TabId> {
        while let Some(closed) = self.closed.pop() {
            if self.tab(closed.record.id).is_none() {
                return Some(self.restore_tab(closed));
            }
        }
        None
    }

    /// Undo closing one specific tab (e.g. from its toast). Its entry leaves
    /// the recently-closed stack; if it is open again already, nothing is
    /// added and its id is returned.
    pub fn undo_close(&mut self, closed: ClosedTab) -> TabId {
        let id = closed.record.id;
        self.closed.retain(|c| c.record.id != id);
        self.restore_tab(closed)
    }

    /// Put a closed tab back where it was (as close as still possible).
    /// Restoring a tab that is already open does nothing and returns its id.
    pub fn restore_tab(&mut self, closed: ClosedTab) -> TabId {
        if self.tab(closed.record.id).is_some() {
            return closed.record.id;
        }
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

    /// The tab that should become selected if `id` goes away: the next
    /// visible tab in sidebar order, else the previous one. A tab hidden in
    /// a collapsed folder hands over to the nearest visible tab.
    pub fn neighbor(&self, id: TabId) -> Option<TabId> {
        let space = self.space(self.space_of(id)?)?;
        let order = space.ordered();
        let visible: std::collections::HashSet<TabId> = space.visible().into_iter().collect();
        let i = order.iter().position(|t| *t == id)?;
        let usable = |t: &&TabId| **t != id && visible.contains(*t);
        order[i + 1..]
            .iter()
            .find(usable)
            .or_else(|| order[..i].iter().rev().find(usable))
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

    /// Drop a tab onto the row (or pinned tile) at `slot`: it takes that
    /// row's place. Within one list the target shifts towards where the
    /// dragged tab came from, so dropping on the row right below moves the
    /// tab down by one; from another list it lands before the target.
    pub fn move_tab_onto(&mut self, id: TabId, space: SpaceId, slot: Slot) -> bool {
        match self.location(id) {
            Some(from) if from.space == space && from.slot.same_list(&slot) && from.slot.index() < slot.index() => {
                self.move_tab(id, space, slot.with_index(slot.index() + 1))
            }
            _ => self.move_tab(id, space, slot),
        }
    }

    /// Move a tab to `slot` in `space`. Moving down within the same list
    /// accounts for the tab's own removal, so "drop before index i" works.
    /// Leaving the space dissolves the tab's split pair.
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
            if from.space != space {
                s.splits.retain(|g| !g.contains(id));
                if was_selected {
                    s.selected = neighbor;
                }
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
        self.cycle_from(self.selected(), delta)
    }

    /// The tab `delta` steps from `from` (e.g. the focused split pane),
    /// wrapping, among the active space's visible tabs.
    pub fn cycle_from(&self, from: Option<TabId>, delta: isize) -> Option<TabId> {
        let order = self.active().visible();
        if order.is_empty() {
            return None;
        }
        let n = order.len() as isize;
        let current = from
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
        let space = Space::new(name);
        let id = space.id;
        self.spaces.push(space);
        id
    }

    /// Default name for the next new space: "Space N".
    pub fn next_space_name(&self) -> String {
        format!("Space {}", self.spaces.len() + 1)
    }

    /// Delete a space and its tabs, returning everything needed to undo it.
    /// Refuses to delete the last space. Falls back to the previous space if
    /// it was active.
    pub fn delete_space(&mut self, id: SpaceId) -> Option<DeletedSpace> {
        if self.spaces.len() <= 1 {
            return None;
        }
        let index = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(index);
        let doomed: std::collections::HashSet<TabId> = space.ordered().into_iter().collect();
        let (tabs, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.tabs)
            .into_iter()
            .partition(|t| doomed.contains(&t.id));
        self.tabs = kept;
        self.reindex();
        let was_active = self.active_space == id;
        if was_active {
            self.active_space = self.spaces[index.saturating_sub(1).min(self.spaces.len() - 1)].id;
        }
        Some(DeletedSpace {
            space,
            index,
            tabs,
            was_active,
        })
    }

    /// Undo [`Workspace::delete_space`]: the space returns at its old place
    /// in the switcher with its tabs, and becomes active again if it was.
    /// Tabs that were reopened elsewhere meanwhile stay where they are.
    pub fn restore_space(&mut self, deleted: DeletedSpace) -> SpaceId {
        let DeletedSpace {
            mut space,
            index,
            tabs,
            was_active,
        } = deleted;
        if self.space(space.id).is_some() {
            space.id = Uuid::new_v4();
        }
        let restored: std::collections::HashSet<TabId> = tabs
            .iter()
            .map(|t| t.id)
            .filter(|id| self.tab(*id).is_none())
            .collect();
        self.tabs.extend(tabs.into_iter().filter(|t| restored.contains(&t.id)));
        self.reindex();
        space.pinned.retain(|t| restored.contains(t));
        space.loose.retain(|t| restored.contains(t));
        for f in &mut space.folders {
            f.tab_ids.retain(|t| restored.contains(t));
        }
        space.splits.retain(|g| g.tabs.iter().all(|t| restored.contains(t)));
        if space.selected.is_some_and(|s| !restored.contains(&s)) {
            space.selected = None;
        }
        let id = space.id;
        self.spaces.insert(index.min(self.spaces.len()), space);
        if was_active {
            self.active_space = id;
        }
        id
    }

    // ---- Split view -----------------------------------------------------------

    /// The split pair `tab` belongs to, if any.
    pub fn split_of(&self, tab: TabId) -> Option<&SplitGroup> {
        self.space(self.space_of(tab)?)?.split_of(tab)
    }

    /// Pair `left` and `right` into a split view. Both must be different
    /// tabs in the same space; any pair either was in is dissolved. Unless
    /// one of them is pinned, `right` moves next to `left` in the sidebar so
    /// the pair shows as one row.
    pub fn create_split(&mut self, left: TabId, right: TabId) -> bool {
        let (Some(l), Some(r)) = (self.location(left), self.location(right)) else {
            return false;
        };
        if left == right || l.space != r.space {
            return false;
        }
        if !l.slot.is_pinned() && !r.slot.is_pinned() {
            self.move_tab(right, l.space, l.slot.with_index(l.slot.index() + 1));
        }
        let Some(space) = self.space_mut(l.space) else {
            return false;
        };
        space.splits.retain(|g| !g.contains(left) && !g.contains(right));
        space.splits.push(SplitGroup::new(left, right));
        true
    }

    /// Dissolve the split pair `tab` belongs to. Returns whether there was one.
    pub fn remove_split(&mut self, tab: TabId) -> bool {
        let Some(space) = self.space_of(tab).and_then(|s| self.space_mut(s)) else {
            return false;
        };
        let before = space.splits.len();
        space.splits.retain(|g| !g.contains(tab));
        space.splits.len() != before
    }

    /// Set the left pane's width share of `tab`'s split pair.
    pub fn set_split_ratio(&mut self, tab: TabId, ratio: f32) -> bool {
        let Some(space) = self.space_of(tab).and_then(|s| self.space_mut(s)) else {
            return false;
        };
        match space.splits.iter_mut().find(|g| g.contains(tab)) {
            Some(g) => {
                g.ratio = ratio;
                g.ratio = g.clamped_ratio();
                true
            }
            None => false,
        }
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
        let deleted = ws.delete_space(work).unwrap();
        assert_eq!(deleted.tabs.len(), 1);
        assert_eq!(ws.active_space, personal);
        assert!(ws.tab(ids[1]).is_none());
        assert!(ws.delete_space(personal).is_none(), "last space is kept");
    }

    #[test]
    fn delete_space_undo_restores_tabs_and_position() {
        let (mut ws, ids) = ws_with(3);
        let personal = ws.active_space;
        let work = ws.create_space("Work");
        let play = ws.create_space("Play");
        ws.move_to_space(ids[1], work);
        ws.move_to_space(ids[2], work);
        ws.pin(ids[2]);
        ws.create_split(ids[1], ids[2]);
        ws.select(ids[1]);
        let deleted = ws.delete_space(work).unwrap();
        assert_eq!(ws.active_space, personal);
        assert_eq!(ws.tabs.len(), 1);
        let back = ws.restore_space(deleted);
        assert_eq!(back, work);
        assert_eq!(ws.spaces.iter().map(|s| s.id).collect::<Vec<_>>(), vec![personal, work, play]);
        assert_eq!(ws.active_space, work);
        assert_eq!(ws.selected(), Some(ids[1]));
        assert!(ws.is_pinned(ids[2]));
        assert_eq!(ws.split_of(ids[1]).map(|g| g.tabs), Some([ids[1], ids[2]]));
        assert_eq!(ws.tab(ids[2]).unwrap().url, "https://t2.test");
    }

    #[test]
    fn restore_space_skips_tabs_reopened_meanwhile() {
        let (mut ws, ids) = ws_with(2);
        let work = ws.create_space("Work");
        ws.move_to_space(ids[1], work);
        let deleted = ws.delete_space(work).unwrap();
        // The tab came back some other way (e.g. a stale undo).
        ws.add_tab(deleted.tabs[0].clone(), None, Slot::Loose(0));
        ws.restore_space(deleted);
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.tab_count(work), 0);
    }

    #[test]
    fn add_tab_never_duplicates_an_id() {
        let (mut ws, ids) = ws_with(1);
        let copy = ws.tab(ids[0]).unwrap().clone();
        let new_id = ws.add_tab(copy, None, Slot::Loose(1));
        assert_ne!(new_id, ids[0]);
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.ordered(), vec![ids[0], new_id]);
    }

    #[test]
    fn restoring_twice_does_not_duplicate() {
        let (mut ws, ids) = ws_with(3);
        let closed = ws.close_tab(ids[1]).unwrap();
        // Ctrl+Shift+T restores it…
        assert_eq!(ws.reopen_closed(), Some(ids[1]));
        // …then the still-visible toast's Undo is clicked.
        assert_eq!(ws.undo_close(closed), ids[1]);
        assert_eq!(ws.tabs.len(), 3);
        assert_eq!(ws.ordered(), ids);
        assert_eq!(ws.reopen_closed(), None);
    }

    #[test]
    fn undo_removes_the_matching_stack_entry() {
        let (mut ws, ids) = ws_with(3);
        let a = ws.close_tab(ids[0]).unwrap();
        ws.close_tab(ids[1]).unwrap();
        // Undo A's (older) toast: B must stay reopenable, A must not come twice.
        assert_eq!(ws.undo_close(a), ids[0]);
        assert_eq!(ws.reopen_closed(), Some(ids[1]));
        assert_eq!(ws.reopen_closed(), None);
        assert_eq!(ws.tabs.len(), 3);
    }

    #[test]
    fn reopen_skips_tabs_that_are_open_again() {
        let (mut ws, ids) = ws_with(3);
        let a = ws.close_tab(ids[0]).unwrap();
        ws.close_tab(ids[1]).unwrap();
        ws.restore_tab(a);
        ws.closed.push(ws.closed[0].clone()); // a stale duplicate entry
        assert_eq!(ws.reopen_closed(), Some(ids[1]));
        assert_eq!(ws.tabs.len(), 3);
    }

    #[test]
    fn closed_stack_is_capped_and_persisted() {
        let (mut ws, _) = ws_with(MAX_CLOSED_TABS + 3);
        for id in ws.ordered() {
            ws.close_tab(id);
        }
        assert_eq!(ws.closed.len(), MAX_CLOSED_TABS);
        let json = serde_json::to_string(&ws).unwrap();
        let mut back: Workspace = serde_json::from_str(&json).unwrap();
        back.repair();
        assert_eq!(back.closed.len(), MAX_CLOSED_TABS);
        assert!(back.reopen_closed().is_some());
    }

    #[test]
    fn repair_drops_duplicate_records() {
        let (mut ws, ids) = ws_with(2);
        let dup = ws.tab(ids[0]).unwrap().clone();
        ws.tabs.push(dup);
        ws.active_mut().loose.push(ids[0]);
        ws.repair();
        assert_eq!(ws.tabs.len(), 2);
        assert_eq!(ws.ordered(), ids);
        // Removing the tab removes it for good; nothing is re-adopted.
        ws.remove_tab(ids[0]);
        ws.repair();
        assert_eq!(ws.ordered(), vec![ids[1]]);
    }

    #[test]
    fn neighbor_of_hidden_tab_is_nearest_visible() {
        let (mut ws, ids) = ws_with(3);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[1]]).unwrap();
        ws.update_folder(f, |f| f.is_expanded = false);
        ws.select(ids[1]);
        // Order: [t1 (hidden in folder), t0, t2].
        assert_eq!(ws.neighbor(ids[1]), Some(ids[0]));
        ws.remove_tab(ids[1]);
        assert_eq!(ws.selected(), Some(ids[0]));
    }

    #[test]
    fn neighbor_skips_collapsed_folders() {
        let (mut ws, ids) = ws_with(3);
        let space = ws.active_space;
        ws.pin(ids[0]);
        let f = ws.create_folder(space, "Work", &[ids[1]]).unwrap();
        ws.update_folder(f, |f| f.is_expanded = false);
        // Order: [t0 pinned, t1 hidden, t2]: t0's neighbour is t2, not t1.
        assert_eq!(ws.neighbor(ids[0]), Some(ids[2]));
        assert_eq!(ws.neighbor(ids[2]), Some(ids[0]));
    }

    #[test]
    fn new_tabs_go_to_top_of_loose_list() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[0]]).unwrap();
        ws.select(ids[0]);
        let n = ws.add_new_tab(TabRecord::new("https://new.test"));
        assert_eq!(ws.active().loose, vec![n, ids[1]]);
        assert_eq!(ws.active().folder(f).unwrap().tab_ids, vec![ids[0]]);
    }

    #[test]
    fn child_tabs_follow_opener_in_order() {
        let (mut ws, ids) = ws_with(3);
        let a = ws.add_child_tab(TabRecord::new("https://a.test"), ids[0], None);
        let b = ws.add_child_tab(TabRecord::new("https://b.test"), ids[0], Some(a));
        assert_eq!(ws.ordered(), vec![ids[0], a, b, ids[1], ids[2]]);
        // A stale anchor (closed) falls back to right after the opener.
        ws.remove_tab(b);
        let c = ws.add_child_tab(TabRecord::new("https://c.test"), ids[0], Some(b));
        assert_eq!(ws.ordered(), vec![ids[0], c, a, ids[1], ids[2]]);
    }

    #[test]
    fn child_tabs_of_pinned_or_folder_tabs_stay_out_of_collections() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[0]]).unwrap();
        ws.pin(ids[1]);
        let a = ws.add_child_tab(TabRecord::new("https://a.test"), ids[0], None);
        assert_eq!(ws.active().folder(f).unwrap().tab_ids, vec![ids[0]]);
        assert_eq!(ws.active().loose, vec![a]);
        let b = ws.add_child_tab(TabRecord::new("https://b.test"), ids[0], Some(a));
        assert_eq!(ws.active().loose, vec![a, b]);
        let c = ws.add_child_tab(TabRecord::new("https://c.test"), ids[1], None);
        assert_eq!(ws.active().pinned, vec![ids[1]]);
        assert_eq!(ws.active().loose, vec![c, a, b]);
    }

    #[test]
    fn drop_onto_row_takes_its_place() {
        let (mut ws, ids) = ws_with(4);
        let space = ws.active_space;
        // Onto the row right below: moves down by one.
        ws.move_tab_onto(ids[0], space, Slot::Loose(1));
        assert_eq!(ws.ordered(), vec![ids[1], ids[0], ids[2], ids[3]]);
        // Onto a row above: lands before it.
        ws.move_tab_onto(ids[3], space, Slot::Loose(1));
        assert_eq!(ws.ordered(), vec![ids[1], ids[3], ids[0], ids[2]]);
        // Onto the last row: becomes last.
        ws.move_tab_onto(ids[1], space, Slot::Loose(3));
        assert_eq!(ws.ordered(), vec![ids[3], ids[0], ids[2], ids[1]]);
        // Onto itself: nothing.
        ws.move_tab_onto(ids[0], space, Slot::Loose(1));
        assert_eq!(ws.ordered(), vec![ids[3], ids[0], ids[2], ids[1]]);
    }

    #[test]
    fn drop_onto_pinned_tile_and_pinned_end() {
        let (mut ws, ids) = ws_with(4);
        let space = ws.active_space;
        for id in &ids[..3] {
            ws.pin(*id);
        }
        ws.move_tab_onto(ids[0], space, Slot::Pinned(1));
        assert_eq!(ws.active().pinned, vec![ids[1], ids[0], ids[2]]);
        // A loose tab dropped on a tile lands before it.
        ws.move_tab_onto(ids[3], space, Slot::Pinned(0));
        assert_eq!(ws.active().pinned, vec![ids[3], ids[1], ids[0], ids[2]]);
        // Dropping past the last tile pins at the end.
        let end = ws.active().pinned.len();
        ws.move_tab(ids[3], space, Slot::Pinned(end));
        assert_eq!(ws.active().pinned, vec![ids[1], ids[0], ids[2], ids[3]]);
    }

    #[test]
    fn split_pairs_are_adjacent_and_dissolve_with_their_tabs() {
        let (mut ws, ids) = ws_with(4);
        assert!(ws.create_split(ids[0], ids[3]));
        assert_eq!(ws.ordered(), vec![ids[0], ids[3], ids[1], ids[2]]);
        assert_eq!(ws.split_of(ids[3]).unwrap().partner(ids[3]), Some(ids[0]));
        // Re-pairing a member replaces its old pair.
        assert!(ws.create_split(ids[1], ids[3]));
        assert!(ws.split_of(ids[0]).is_none());
        assert_eq!(ws.split_of(ids[1]).unwrap().tabs, [ids[1], ids[3]]);
        // Closing a member dissolves the pair.
        ws.close_tab(ids[3]);
        assert!(ws.split_of(ids[1]).is_none());
        assert!(!ws.create_split(ids[1], ids[1]));
    }

    #[test]
    fn cycling_steps_through_both_split_panes() {
        let (mut ws, ids) = ws_with(3);
        ws.create_split(ids[0], ids[1]);
        // A shown pair keeps its left tab selected; the focused pane is the anchor.
        ws.select(ids[0]);
        assert_eq!(ws.cycle_from(Some(ids[0]), 1), Some(ids[1]));
        assert_eq!(ws.cycle_from(Some(ids[1]), 1), Some(ids[2]));
        assert_eq!(ws.cycle_from(Some(ids[1]), -1), Some(ids[0]));
        assert_eq!(ws.cycle_from(None, 1), Some(ids[0]));
    }

    #[test]
    fn split_leaves_with_a_tab_moved_to_another_space() {
        let (mut ws, ids) = ws_with(2);
        let work = ws.create_space("Work");
        ws.create_split(ids[0], ids[1]);
        assert!(!ws.create_split(ids[0], Uuid::new_v4()));
        ws.move_to_space(ids[1], work);
        assert!(ws.split_of(ids[0]).is_none());
        assert!(ws.split_of(ids[1]).is_none());
    }

    #[test]
    fn split_ratio_is_clamped_and_persisted() {
        let (mut ws, ids) = ws_with(2);
        ws.create_split(ids[0], ids[1]);
        ws.set_split_ratio(ids[1], 0.95);
        assert_eq!(ws.split_of(ids[0]).unwrap().ratio, 1.0 - SPLIT_MIN_RATIO);
        ws.set_split_ratio(ids[0], 0.3);
        let json = serde_json::to_string(&ws).unwrap();
        let mut back: Workspace = serde_json::from_str(&json).unwrap();
        back.repair();
        assert_eq!(back.split_of(ids[1]).unwrap().ratio, 0.3);
        assert!(ws.remove_split(ids[0]));
        assert!(!ws.remove_split(ids[0]));
    }

    #[test]
    fn repair_drops_broken_split_pairs() {
        let (mut ws, ids) = ws_with(3);
        let space = ws.active_mut();
        space.splits.push(SplitGroup::new(ids[0], Uuid::new_v4()));
        space.splits.push(SplitGroup::new(ids[1], ids[1]));
        space.splits.push(SplitGroup::new(ids[1], ids[2]));
        space.splits.push(SplitGroup::new(ids[2], ids[0]));
        ws.repair();
        assert_eq!(ws.active().splits, vec![SplitGroup::new(ids[1], ids[2])]);
    }

    #[test]
    fn repair_keeps_a_valid_pair_after_a_dropped_overlapping_one() {
        let (mut ws, ids) = ws_with(4);
        let space = ws.active_mut();
        space.splits.push(SplitGroup::new(ids[0], ids[1]));
        // Overlaps the first pair on ids[1]: dropped, and must not mark ids[2] as paired.
        space.splits.push(SplitGroup::new(ids[2], ids[1]));
        space.splits.push(SplitGroup::new(ids[2], ids[3]));
        ws.repair();
        assert_eq!(
            ws.active().splits,
            vec![SplitGroup::new(ids[0], ids[1]), SplitGroup::new(ids[2], ids[3])]
        );
    }

    #[test]
    fn records_from_older_sessions_load() {
        let json = r#"{"tabs":[{"id":"7d1b6a4e-3f4c-4a43-9a51-0b8f4b4a8a10","url":"https://a.test"}],
            "spaces":[{"id":"1d1b6a4e-3f4c-4a43-9a51-0b8f4b4a8a10","name":"P","loose":["7d1b6a4e-3f4c-4a43-9a51-0b8f4b4a8a10"]}],
            "active_space":"1d1b6a4e-3f4c-4a43-9a51-0b8f4b4a8a10"}"#;
        let mut ws: Workspace = serde_json::from_str(json).unwrap();
        ws.repair();
        assert_eq!(ws.tabs[0].zoom_level, 0.0);
        assert!(ws.closed.is_empty());
        assert!(ws.active().splits.is_empty());
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
