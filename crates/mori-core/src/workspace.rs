//! Tabs, spaces, favorites, and folders.
//!
//! A [`Workspace`] is the persisted shape of the sidebar, organised like Arc
//! (Zen calls the same tiers Essentials, Pinned and the tab list):
//!
//! - **Favorites**: one grid of tabs shared by every space, at the top of
//!   the sidebar. A favorite belongs to no space; it shows in all of them.
//! - **Pinned**: per space, rows above the divider: pinned tabs and
//!   (nested) folders. Pinned tabs and favorites remember the address they
//!   were saved with ([`TabRecord::pinned_url`]) and can go back to it.
//! - **Today**: per space, the loose tabs below the divider. They are the
//!   ones auto-archive and "Clear" put away.
//!
//! Live browser state (loading, page views) lives in the UI; this module
//! only owns membership, order, and selection, so every rule here is
//! unit-testable.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::theme::{GradientDot, GradientTheme, Rgba};

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
    /// Last known vertical scroll offset in CSS pixels, restored when a
    /// slept or restored tab loads again.
    #[serde(default)]
    pub scroll_y: f64,
    /// The address a favorite or pinned tab was saved with. Navigating away
    /// keeps it, so the tab can go back ("Back to pinned page"), and closing
    /// the tab resets it there. Loose tabs have none.
    #[serde(default)]
    pub pinned_url: Option<String>,
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
            scroll_y: 0.0,
            pinned_url: None,
        }
    }

    /// Custom title if set, else the page title, else the address without
    /// its scheme (a page not loaded yet), else "New Tab".
    pub fn display_title(&self) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        if let Some(t) = self.custom_title.as_deref()
            && !t.trim().is_empty()
        {
            return Cow::Borrowed(t);
        }
        if !self.title.trim().is_empty() {
            return Cow::Borrowed(&self.title);
        }
        let placeholder = crate::omnibox::placeholder_title(&self.url);
        if placeholder.is_empty() { Cow::Borrowed("New Tab") } else { Cow::Owned(placeholder) }
    }

    /// Whether the tab has navigated away from the address it was pinned
    /// with (ignoring the fragment and a trailing `/`).
    pub fn left_pinned_url(&self) -> bool {
        self.pinned_url
            .as_deref()
            .is_some_and(|p| !crate::omnibox::same_page(p, &self.url))
    }
}

/// Whether `url` is one of Mori's own pages (or blank), which are never
/// worth remembering as a pinned address.
pub fn is_internal_url(url: &str) -> bool {
    url.is_empty() || url.starts_with("mori://") || url == "about:blank"
}

/// One entry of a space's pinned section: a tab or a folder. Tabs are
/// stored as bare ids, so the pinned grid of older sessions (a plain id
/// list) loads as pinned rows unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Item {
    Tab(TabId),
    Folder(Folder),
}

impl Item {
    fn is_tab(&self, tab: TabId) -> bool {
        matches!(self, Item::Tab(t) if *t == tab)
    }

    fn is_folder(&self, folder: FolderId) -> bool {
        matches!(self, Item::Folder(f) if f.id == folder)
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
    /// Tabs and subfolders in sidebar order. Older sessions stored a flat
    /// `tab_ids` list, which loads as tab items.
    #[serde(default, alias = "tab_ids")]
    pub items: Vec<Item>,
}

impl Folder {
    pub fn new(name: impl Into<String>) -> Self {
        Folder {
            id: Uuid::new_v4(),
            name: name.into(),
            symbol: default_folder_symbol(),
            is_expanded: true,
            items: Vec::new(),
        }
    }

    /// Every tab inside, subfolders included, in sidebar order.
    pub fn tabs(&self) -> Vec<TabId> {
        let mut out = Vec::new();
        collect_tabs(&self.items, false, &mut out);
        out
    }

    /// How many folder levels this folder spans (1 without subfolders).
    pub fn height(&self) -> usize {
        1 + self
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Folder(f) => Some(f.height()),
                Item::Tab(_) => None,
            })
            .max()
            .unwrap_or(0)
    }
}

/// How deep folders may nest: a top-level folder is at depth 1.
pub const MAX_FOLDER_DEPTH: usize = 3;

fn default_folder_symbol() -> String {
    "folder".into()
}

fn collect_tabs(items: &[Item], only_visible: bool, out: &mut Vec<TabId>) {
    for item in items {
        match item {
            Item::Tab(t) => out.push(*t),
            Item::Folder(f) => {
                if !only_visible || f.is_expanded {
                    collect_tabs(&f.items, only_visible, out);
                }
            }
        }
    }
}

fn find_folder(items: &[Item], id: FolderId) -> Option<&Folder> {
    items.iter().find_map(|i| match i {
        Item::Folder(f) if f.id == id => Some(f),
        Item::Folder(f) => find_folder(&f.items, id),
        Item::Tab(_) => None,
    })
}

fn find_folder_mut(items: &mut [Item], id: FolderId) -> Option<&mut Folder> {
    for item in items {
        if let Item::Folder(f) = item {
            if f.id == id {
                return Some(f);
            }
            if let Some(found) = find_folder_mut(&mut f.items, id) {
                return Some(found);
            }
        }
    }
    None
}

/// The folder holding `tab` (`None` = this list) and its index there.
fn find_tab(items: &[Item], tab: TabId, parent: Option<FolderId>) -> Option<(Option<FolderId>, usize)> {
    for (i, item) in items.iter().enumerate() {
        match item {
            Item::Tab(t) if *t == tab => return Some((parent, i)),
            Item::Folder(f) => {
                if let Some(found) = find_tab(&f.items, tab, Some(f.id)) {
                    return Some(found);
                }
            }
            Item::Tab(_) => {}
        }
    }
    None
}

/// The folder holding folder `id` (`None` = this list) and its index there.
fn find_folder_slot(items: &[Item], id: FolderId, parent: Option<FolderId>) -> Option<(Option<FolderId>, usize)> {
    for (i, item) in items.iter().enumerate() {
        if let Item::Folder(f) = item {
            if f.id == id {
                return Some((parent, i));
            }
            if let Some(found) = find_folder_slot(&f.items, id, Some(f.id)) {
                return Some(found);
            }
        }
    }
    None
}

/// The ids of the folders enclosing `target` (outermost first), not
/// including it. `None` if `target` isn't in `items`.
fn ancestors_of(items: &[Item], matches: &dyn Fn(&Item) -> bool) -> Option<Vec<FolderId>> {
    for item in items {
        if matches(item) {
            return Some(Vec::new());
        }
        if let Item::Folder(f) = item
            && let Some(mut path) = ancestors_of(&f.items, matches)
        {
            path.insert(0, f.id);
            return Some(path);
        }
    }
    None
}

fn retain_tabs(items: &mut Vec<Item>, keep: &mut dyn FnMut(&TabId) -> bool) {
    items.retain_mut(|item| match item {
        Item::Tab(t) => keep(t),
        Item::Folder(f) => {
            retain_tabs(&mut f.items, keep);
            true
        }
    });
}

fn all_folders<'a>(items: &'a [Item], depth: usize, out: &mut Vec<(&'a Folder, usize)>) {
    for item in items {
        if let Item::Folder(f) = item {
            out.push((f, depth));
            all_folders(&f.items, depth + 1, out);
        }
    }
}

/// The symbol a space starts with; the switcher draws it as a plain dot.
pub const DEFAULT_SPACE_SYMBOL: &str = "circle";

fn default_space_symbol() -> String {
    DEFAULT_SPACE_SYMBOL.into()
}

/// Whether a space symbol is an emoji (drawn as text) rather than the name
/// of a Lucide icon.
pub fn is_emoji_symbol(symbol: &str) -> bool {
    !symbol.is_ascii()
}

/// Symbols (Lucide names) handed to new spaces in turn, so every space is
/// recognisable in the switcher. The first space keeps the plain default.
pub const NEW_SPACE_SYMBOLS: [&str; 10] = [
    "briefcase", "code", "book-open", "palette", "music", "leaf", "star", "heart", "flame", "globe",
];

/// Colour pairs (`0xRRGGBB`, primary first) handed to new spaces in turn,
/// so spaces look distinct the way Arc's do. Each pair is deep enough that
/// the chrome stays dark with light text, whatever the pair.
pub const NEW_SPACE_COLORS: [(u32, u32); 8] = [
    (0x6E5BD8, 0x9B5BD8), // violet
    (0x3577D4, 0x5B5BD8), // blue
    (0x1F9488, 0x2F78A8), // teal
    (0xD9702E, 0xC94E5A), // orange
    (0xD24E7C, 0x9C4ED2), // pink
    (0x4F9A3C, 0x2C8C78), // green
    (0xC48A1E, 0xC45A1E), // amber
    (0x8E5BD0, 0xC85BA8), // purple
];

/// The two-tone gradient for a colour pair from [`NEW_SPACE_COLORS`].
pub fn space_theme_for_colors((primary, secondary): (u32, u32)) -> GradientTheme {
    GradientTheme {
        dots: vec![
            GradientDot::from_rgb(Rgba::hex(primary), true),
            GradientDot::from_rgb(Rgba::hex(secondary), false),
        ],
        opacity: 0.5,
        texture: 0.0,
        scheme_override: Default::default(),
        preset_id: None,
    }
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
#[serde(from = "SpaceRepr")]
pub struct Space {
    pub id: SpaceId,
    pub name: String,
    pub symbol: String,
    pub theme: GradientTheme,
    /// The pinned section above the divider: pinned tabs and folders.
    pub pinned: Vec<Item>,
    /// Today's tabs below the divider, in sidebar order.
    pub loose: Vec<TabId>,
    /// The tab on screen in this space; may be a favorite.
    pub selected: Option<TabId>,
    /// Remembered split-view pairs.
    pub splits: Vec<SplitGroup>,
}

/// What a saved space looks like on disk, any version.
#[derive(Deserialize)]
struct SpaceRepr {
    id: SpaceId,
    name: String,
    #[serde(default = "default_space_symbol")]
    symbol: String,
    #[serde(default = "GradientTheme::none")]
    theme: GradientTheme,
    #[serde(default)]
    pinned: Vec<Item>,
    /// Sessions before version 4 kept folders in a list of their own,
    /// shown right after the pins.
    #[serde(default)]
    folders: Vec<Folder>,
    #[serde(default)]
    loose: Vec<TabId>,
    #[serde(default)]
    selected: Option<TabId>,
    #[serde(default)]
    splits: Vec<SplitGroup>,
}

impl From<SpaceRepr> for Space {
    fn from(r: SpaceRepr) -> Space {
        let mut pinned = r.pinned;
        pinned.extend(r.folders.into_iter().map(Item::Folder));
        Space {
            id: r.id,
            name: r.name,
            symbol: r.symbol,
            theme: r.theme,
            pinned,
            loose: r.loose,
            selected: r.selected,
            splits: r.splits,
        }
    }
}

impl Space {
    pub fn new(name: impl Into<String>) -> Self {
        Space {
            id: Uuid::new_v4(),
            name: name.into(),
            symbol: default_space_symbol(),
            theme: GradientTheme::none(),
            pinned: Vec::new(),
            loose: Vec::new(),
            selected: None,
            splits: Vec::new(),
        }
    }

    pub fn contains(&self, tab: TabId) -> bool {
        self.loose.contains(&tab) || find_tab(&self.pinned, tab, None).is_some()
    }

    /// Every tab of the space in sidebar order: pinned rows (folders in
    /// place), then loose tabs. Favorites aren't part of any space.
    pub fn ordered(&self) -> Vec<TabId> {
        let mut out = Vec::new();
        collect_tabs(&self.pinned, false, &mut out);
        out.extend(&self.loose);
        out
    }

    /// Tabs currently visible in the sidebar (collapsed folders hide theirs).
    pub fn visible(&self) -> Vec<TabId> {
        let mut out = Vec::new();
        collect_tabs(&self.pinned, true, &mut out);
        out.extend(&self.loose);
        out
    }

    pub fn folder(&self, id: FolderId) -> Option<&Folder> {
        find_folder(&self.pinned, id)
    }

    /// Every folder with its depth (1 = top level), depth-first.
    pub fn all_folders(&self) -> Vec<(&Folder, usize)> {
        let mut out = Vec::new();
        all_folders(&self.pinned, 1, &mut out);
        out
    }

    /// How deep `folder` sits (1 = top level).
    pub fn folder_depth(&self, folder: FolderId) -> Option<usize> {
        ancestors_of(&self.pinned, &|i| i.is_folder(folder)).map(|path| path.len() + 1)
    }

    /// `folder` and the folders around it, outermost first.
    pub fn folder_trail(&self, folder: FolderId) -> Vec<&Folder> {
        let Some(path) = ancestors_of(&self.pinned, &|i| i.is_folder(folder)) else {
            return Vec::new();
        };
        path.into_iter()
            .chain([folder])
            .filter_map(|id| self.folder(id))
            .collect()
    }

    /// The split pair `tab` belongs to, if any.
    pub fn split_of(&self, tab: TabId) -> Option<&SplitGroup> {
        self.splits.iter().find(|g| g.contains(tab))
    }

    fn folder_mut(&mut self, id: FolderId) -> Option<&mut Folder> {
        find_folder_mut(&mut self.pinned, id)
    }

    /// The item list a pinned slot points into: the top level or a folder.
    fn items_mut(&mut self, folder: Option<FolderId>) -> Option<&mut Vec<Item>> {
        match folder {
            None => Some(&mut self.pinned),
            Some(f) => self.folder_mut(f).map(|f| &mut f.items),
        }
    }

    fn items(&self, folder: Option<FolderId>) -> Option<&Vec<Item>> {
        match folder {
            None => Some(&self.pinned),
            Some(f) => self.folder(f).map(|f| &f.items),
        }
    }

    fn slot_of(&self, tab: TabId) -> Option<Slot> {
        if let Some((parent, i)) = find_tab(&self.pinned, tab, None) {
            return Some(match parent {
                None => Slot::Pinned(i),
                Some(f) => Slot::Folder(f, i),
            });
        }
        self.loose.iter().position(|t| *t == tab).map(Slot::Loose)
    }

    /// Open every folder around `target`, so it shows in the sidebar.
    fn reveal(&mut self, matches: &dyn Fn(&Item) -> bool) {
        for fid in ancestors_of(&self.pinned, matches).unwrap_or_default() {
            if let Some(f) = self.folder_mut(fid) {
                f.is_expanded = true;
            }
        }
    }

    /// Keep only the tabs `keep` accepts, wherever they sit.
    fn retain_tabs(&mut self, keep: &mut dyn FnMut(&TabId) -> bool) {
        retain_tabs(&mut self.pinned, keep);
        self.loose.retain(|t| keep(t));
    }
}

/// A position: in the favorites grid, a space's pinned section (top level
/// or inside a folder; the index counts folders too), or its loose list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Slot {
    Pinned(usize),
    Folder(FolderId, usize),
    Loose(usize),
    Favorite(usize),
}

impl Slot {
    /// In a space's pinned section (top level or in a folder).
    pub fn is_pinned(&self) -> bool {
        matches!(self, Slot::Pinned(_) | Slot::Folder(..))
    }

    pub fn is_favorite(&self) -> bool {
        matches!(self, Slot::Favorite(_))
    }

    /// A favorite or pinned position: the tab keeps a pinned address and
    /// closing it only unloads it.
    pub fn is_saved(&self) -> bool {
        !matches!(self, Slot::Loose(_))
    }

    pub fn folder(&self) -> Option<FolderId> {
        match self {
            Slot::Folder(f, _) => Some(*f),
            _ => None,
        }
    }

    /// Whether both slots are in the same list (favorites, the pinned top
    /// level, one folder, or loose).
    pub fn same_list(&self, other: &Slot) -> bool {
        match (self, other) {
            (Slot::Pinned(_), Slot::Pinned(_))
            | (Slot::Loose(_), Slot::Loose(_))
            | (Slot::Favorite(_), Slot::Favorite(_)) => true,
            (Slot::Folder(a, _), Slot::Folder(b, _)) => a == b,
            _ => false,
        }
    }

    /// Position within its list.
    pub fn index(&self) -> usize {
        match self {
            Slot::Pinned(i) | Slot::Folder(_, i) | Slot::Loose(i) | Slot::Favorite(i) => *i,
        }
    }

    pub fn with_index(&self, i: usize) -> Slot {
        match self {
            Slot::Pinned(_) => Slot::Pinned(i),
            Slot::Folder(f, _) => Slot::Folder(*f, i),
            Slot::Loose(_) => Slot::Loose(i),
            Slot::Favorite(_) => Slot::Favorite(i),
        }
    }

    /// The pinned item list this slot is in: `Some(None)` for the top
    /// level, `Some(Some(folder))` for a folder, `None` otherwise.
    fn pinned_list(&self) -> Option<Option<FolderId>> {
        match self {
            Slot::Pinned(_) => Some(None),
            Slot::Folder(f, _) => Some(Some(*f)),
            _ => None,
        }
    }
}

/// Where a tab lives. A favorite reports the active space, the one it is
/// shown in.
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

/// How many favorites the grid holds (as Zen's Essentials).
pub const MAX_FAVORITES: usize = 12;

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

/// What an insertion goes before, so moving entries out of the same list
/// can't shift the target.
#[derive(Clone, Copy, PartialEq)]
enum Anchor {
    Tab(TabId),
    Folder(FolderId),
    End,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    #[serde(default = "legacy_version")]
    pub version: u32,
    pub tabs: Vec<TabRecord>,
    pub spaces: Vec<Space>,
    pub active_space: SpaceId,
    /// Favorites, shared by every space, in grid order.
    #[serde(default)]
    pub favorites: Vec<TabId>,
    /// Recently closed tabs, oldest first, for reopen-closed-tab.
    #[serde(default)]
    pub closed: Vec<ClosedTab>,
    /// Not persisted: lookup from id to index in `tabs`.
    #[serde(skip)]
    index: HashMap<TabId, usize>,
}

/// Version 4 introduced favorites, pinned rows with remembered addresses,
/// and nested folders.
pub const SESSION_VERSION: u32 = 4;

/// Sessions saved before the version field existed.
fn legacy_version() -> u32 {
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
            version: SESSION_VERSION,
            active_space: space.id,
            tabs: Vec::new(),
            spaces: vec![space],
            favorites: Vec::new(),
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

    /// Restore invariants after loading untrusted data or an older session:
    /// drop duplicate tab records, dangling ids and duplicate memberships,
    /// adopt orphan tabs, drop broken split pairs, give saved tabs their
    /// pinned address, and keep at least one space.
    pub fn repair(&mut self) {
        if self.spaces.is_empty() {
            self.spaces.push(Space::new("Personal"));
        }
        let mut known = HashSet::new();
        self.tabs.retain(|t| known.insert(t.id));
        let mut seen = HashSet::new();
        self.favorites.retain(|id| known.contains(id) && seen.insert(*id));
        let mut folder_ids = HashSet::new();
        for space in &mut self.spaces {
            space.retain_tabs(&mut |id: &TabId| known.contains(id) && seen.insert(*id));
            fresh_duplicate_folder_ids(&mut space.pinned, &mut folder_ids);
        }
        let favorites: HashSet<TabId> = self.favorites.iter().copied().collect();
        for space in &mut self.spaces {
            if space.selected.is_some_and(|s| !space.contains(s) && !favorites.contains(&s)) {
                space.selected = None;
            }
            let mut paired = HashSet::new();
            let members: HashSet<TabId> = space.ordered().into_iter().chain(favorites.iter().copied()).collect();
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
        let ids: Vec<TabId> = self.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            self.sync_pinned_url(id);
        }
        self.version = SESSION_VERSION;
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
        if let Some(i) = self.favorites.iter().position(|t| *t == tab) {
            return Some(Location {
                space: self.active_space,
                slot: Slot::Favorite(i),
            });
        }
        self.spaces.iter().find_map(|s| {
            s.slot_of(tab).map(|slot| Location {
                space: s.id,
                slot,
            })
        })
    }

    /// The space a tab shows in (the active one for a favorite).
    pub fn space_of(&self, tab: TabId) -> Option<SpaceId> {
        self.location(tab).map(|l| l.space)
    }

    pub fn is_favorite(&self, tab: TabId) -> bool {
        self.favorites.contains(&tab)
    }

    /// In a space's pinned section (top level or in a folder).
    pub fn is_pinned(&self, tab: TabId) -> bool {
        self.location(tab).is_some_and(|l| l.slot.is_pinned())
    }

    /// A favorite or pinned tab: closing it only unloads it.
    pub fn is_saved(&self, tab: TabId) -> bool {
        self.location(tab).is_some_and(|l| l.slot.is_saved())
    }

    pub fn folder_of(&self, tab: TabId) -> Option<FolderId> {
        self.location(tab).and_then(|l| l.slot.folder())
    }

    pub fn find_folder(&self, folder: FolderId) -> Option<(SpaceId, &Folder)> {
        self.spaces
            .iter()
            .find_map(|s| s.folder(folder).map(|f| (s.id, f)))
    }

    /// Tabs visible in `space`'s sidebar in order: favorites, then the
    /// space's own visible tabs.
    pub fn visible_in(&self, space: SpaceId) -> Vec<TabId> {
        let mut out = self.favorites.clone();
        if let Some(s) = self.space(space) {
            out.extend(s.visible());
        }
        out
    }

    /// Every tab shown with `space`: favorites, then the space's tabs.
    pub fn ordered_in(&self, space: SpaceId) -> Vec<TabId> {
        let mut out = self.favorites.clone();
        if let Some(s) = self.space(space) {
            out.extend(s.ordered());
        }
        out
    }

    /// The tabs from `a` to `b` (either order) as the active space's
    /// sidebar shows them, for shift-click selection. Empty if either is
    /// not visible there.
    pub fn visible_range(&self, a: TabId, b: TabId) -> Vec<TabId> {
        let order = self.visible_in(self.active_space);
        let (Some(i), Some(j)) = (order.iter().position(|t| *t == a), order.iter().position(|t| *t == b)) else {
            return Vec::new();
        };
        order[i.min(j)..=i.max(j)].to_vec()
    }

    /// `ids` in the active space's sidebar order (unknown ones dropped).
    pub fn in_sidebar_order(&self, ids: &[TabId]) -> Vec<TabId> {
        self.ordered_in(self.active_space)
            .into_iter()
            .filter(|t| ids.contains(t))
            .collect()
    }

    // ---- Pinned addresses -----------------------------------------------------

    /// A saved tab without a pinned address takes its current one (unless
    /// that is blank or internal); a loose tab forgets it.
    fn sync_pinned_url(&mut self, id: TabId) {
        let saved = self.is_saved(id);
        if let Some(t) = self.tab_mut(id) {
            if !saved {
                t.pinned_url = None;
            } else if t.pinned_url.is_none() && !is_internal_url(&t.url) {
                t.pinned_url = Some(t.url.clone());
            }
        }
    }

    /// The tab's page moved to `url`. A favorite or pinned tab that had no
    /// real address yet (pinned while blank) adopts this one.
    pub fn set_url(&mut self, id: TabId, url: &str) {
        if let Some(t) = self.tab_mut(id) {
            if t.url != url {
                t.scroll_y = 0.0;
            }
            t.url = url.to_string();
        }
        self.sync_pinned_url(id);
    }

    /// Point a saved tab back at its pinned address. Returns the address
    /// when the tab had left it.
    pub fn reset_to_pinned(&mut self, id: TabId) -> Option<String> {
        let t = self.tab_mut(id)?;
        if !t.left_pinned_url() {
            return None;
        }
        let url = t.pinned_url.clone()?;
        t.url = url.clone();
        t.scroll_y = 0.0;
        t.title.clear();
        Some(url)
    }

    /// Make the tab's current address its pinned one.
    pub fn replace_pinned_url(&mut self, id: TabId) -> bool {
        if !self.is_saved(id) {
            return false;
        }
        match self.tab_mut(id) {
            Some(t) if !is_internal_url(&t.url) => {
                t.pinned_url = Some(t.url.clone());
                true
            }
            _ => false,
        }
    }

    // ---- Membership plumbing --------------------------------------------------

    /// Remove `tab` from whichever list holds it.
    fn detach(&mut self, tab: TabId) -> Option<Slot> {
        if let Some(i) = self.favorites.iter().position(|t| *t == tab) {
            self.favorites.remove(i);
            return Some(Slot::Favorite(i));
        }
        for space in &mut self.spaces {
            let Some(slot) = space.slot_of(tab) else { continue };
            match slot {
                Slot::Loose(i) => {
                    space.loose.remove(i);
                }
                _ => {
                    if let Some(items) = slot.pinned_list().and_then(|f| space.items_mut(f)) {
                        items.remove(slot.index());
                    }
                }
            }
            return Some(slot);
        }
        None
    }

    /// The entry of `space`'s list `slot` points at, skipping `moving`
    /// entries; `End` past the last one.
    fn anchor_at(&self, space: SpaceId, slot: Slot, moving_tabs: &[TabId], moving_folder: Option<FolderId>) -> Anchor {
        let skip_tab = |t: &TabId| moving_tabs.contains(t);
        match slot {
            Slot::Favorite(i) => self.favorites.iter().skip(i).find(|t| !skip_tab(t)).map_or(Anchor::End, |t| Anchor::Tab(*t)),
            Slot::Loose(i) => self
                .space(space)
                .and_then(|s| s.loose.iter().skip(i).find(|t| !skip_tab(t)).copied())
                .map_or(Anchor::End, Anchor::Tab),
            _ => {
                let items = slot.pinned_list().and_then(|f| self.space(space)?.items(f));
                items
                    .and_then(|items| {
                        items.iter().skip(slot.index()).find_map(|item| match item {
                            Item::Tab(t) if !skip_tab(t) => Some(Anchor::Tab(*t)),
                            Item::Folder(f) if Some(f.id) != moving_folder => Some(Anchor::Folder(f.id)),
                            _ => None,
                        })
                    })
                    .unwrap_or(Anchor::End)
            }
        }
    }

    /// Insert `entries` (in order) before `anchor` in `space`'s list of
    /// kind `slot`.
    fn insert_at_anchor(&mut self, space: SpaceId, slot: Slot, anchor: Anchor, entries: Vec<Item>) -> bool {
        let position = |items: &[Item]| match anchor {
            Anchor::Tab(t) => items.iter().position(|i| i.is_tab(t)),
            Anchor::Folder(f) => items.iter().position(|i| i.is_folder(f)),
            Anchor::End => None,
        };
        let tab_ids = |entries: &[Item]| -> Vec<TabId> {
            entries
                .iter()
                .filter_map(|e| match e {
                    Item::Tab(t) => Some(*t),
                    Item::Folder(_) => None,
                })
                .collect()
        };
        match slot {
            Slot::Favorite(_) => {
                let at = match anchor {
                    Anchor::Tab(t) => self.favorites.iter().position(|x| *x == t),
                    _ => None,
                }
                .unwrap_or(self.favorites.len());
                let ids = tab_ids(&entries);
                self.favorites.splice(at..at, ids);
            }
            Slot::Loose(_) => {
                let Some(s) = self.space_mut(space) else { return false };
                let at = match anchor {
                    Anchor::Tab(t) => s.loose.iter().position(|x| *x == t),
                    _ => None,
                }
                .unwrap_or(s.loose.len());
                s.loose.splice(at..at, tab_ids(&entries));
            }
            _ => {
                let folder = slot.pinned_list().flatten();
                let Some(s) = self.space_mut(space) else { return false };
                let Some(items) = slot.pinned_list().and_then(|f| s.items_mut(f)) else {
                    return false;
                };
                let at = position(items).unwrap_or(items.len());
                items.splice(at..at, entries);
                if let Some(f) = folder {
                    s.reveal(&|i| i.is_folder(f));
                    if let Some(f) = s.folder_mut(f) {
                        f.is_expanded = true;
                    }
                }
            }
        }
        true
    }

    /// For every space showing one of `tabs`, the tab to show instead if
    /// they leave it: the nearest visible tab not among them.
    fn selection_fallbacks(&self, tabs: &[TabId]) -> Vec<(SpaceId, Option<TabId>)> {
        self.spaces
            .iter()
            .filter_map(|s| {
                let sel = s.selected?;
                tabs.contains(&sel).then(|| (s.id, self.neighbor_in(s.id, sel, tabs)))
            })
            .collect()
    }

    /// Spaces whose selected tab no longer shows in them take their fallback.
    fn settle_selection(&mut self, fallbacks: Vec<(SpaceId, Option<TabId>)>) {
        let favorites: HashSet<TabId> = self.favorites.iter().copied().collect();
        for (space, fallback) in fallbacks {
            if let Some(s) = self.space_mut(space)
                && let Some(sel) = s.selected
                && !s.contains(sel)
                && !favorites.contains(&sel)
            {
                s.selected = fallback;
            }
        }
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
        let space_id = space.filter(|s| self.space(*s).is_some()).unwrap_or(self.active_space);
        self.tabs.push(record);
        self.index.insert(id, self.tabs.len() - 1);
        let slot = match slot {
            Slot::Folder(f, _) if self.space(space_id).and_then(|s| s.folder(f)).is_none() => {
                Slot::Loose(slot.index())
            }
            Slot::Favorite(_) if self.favorites.len() >= MAX_FAVORITES => Slot::Loose(0),
            other => other,
        };
        let anchor = self.anchor_at(space_id, slot, &[], None);
        self.insert_at_anchor(space_id, slot, anchor, vec![Item::Tab(id)]);
        self.sync_pinned_url(id);
        id
    }

    /// Add a tab the user opened (new tab, typed address, external link):
    /// like Arc, it goes to the top of the active space's loose list, never
    /// into a folder, the pinned rows, or the favorites.
    pub fn add_new_tab(&mut self, record: TabRecord) -> TabId {
        self.add_tab(record, None, Slot::Loose(0))
    }

    /// Add a tab opened from `opener` (a link, a duplicate, a promoted peek).
    ///
    /// It goes right below the opener when the opener is loose; tabs opened
    /// from favorites, pinned or foldered tabs go to the top of that space's
    /// loose list, because those are saved collections. `last_child` is the
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

    /// Remove a tab, returning it with its location for undo. Spaces
    /// showing it move to a neighbour, and a split pair it belonged to is
    /// dissolved.
    pub fn remove_tab(&mut self, id: TabId) -> Option<ClosedTab> {
        let location = self.location(id)?;
        let fallbacks = self.selection_fallbacks(&[id]);
        self.detach(id);
        for space in &mut self.spaces {
            space.splits.retain(|g| !g.contains(id));
        }
        self.settle_selection(fallbacks);
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

    /// Put a closed tab back where it was (as close as still possible: a
    /// tab whose folder is gone goes back to the pinned rows). Restoring a
    /// tab that is already open does nothing and returns its id.
    pub fn restore_tab(&mut self, closed: ClosedTab) -> TabId {
        if self.tab(closed.record.id).is_some() {
            return closed.record.id;
        }
        let space = self.space(closed.location.space).map(|s| s.id);
        let slot = match closed.location.slot {
            Slot::Folder(f, _) if space.and_then(|s| self.space(s)?.folder(f)).is_none() => {
                Slot::Pinned(usize::MAX)
            }
            other => other,
        };
        self.add_tab(closed.record, space, slot)
    }

    /// The tab that should become selected if `id` goes away: the next
    /// visible tab in sidebar order (favorites first), else the previous
    /// one. A tab hidden in a collapsed folder hands over to the nearest
    /// visible tab.
    pub fn neighbor(&self, id: TabId) -> Option<TabId> {
        self.neighbor_in(self.space_of(id)?, id, &[])
    }

    /// The tab to show once `id` and every tab in `skip` (a multi-selection
    /// being closed) are gone: the nearest visible tab outside them.
    pub fn neighbor_skipping(&self, id: TabId, skip: &[TabId]) -> Option<TabId> {
        self.neighbor_in(self.space_of(id)?, id, skip)
    }

    /// [`Workspace::neighbor`] as `space` shows it, also skipping `skip`.
    fn neighbor_in(&self, space: SpaceId, id: TabId, skip: &[TabId]) -> Option<TabId> {
        let order = self.ordered_in(space);
        let visible: HashSet<TabId> = self.visible_in(space).into_iter().collect();
        let i = order.iter().position(|t| *t == id)?;
        let usable = |t: &&TabId| **t != id && !skip.contains(*t) && visible.contains(*t);
        order[i + 1..]
            .iter()
            .find(usable)
            .or_else(|| order[..i].iter().rev().find(usable))
            .copied()
    }

    /// Select a tab, switching to its space (a favorite keeps the active
    /// space) and opening the folders around it. Returns false if unknown.
    pub fn select(&mut self, id: TabId) -> bool {
        let Some(space) = self.space_of(id) else {
            return false;
        };
        self.active_space = space;
        if let Some(t) = self.tab_mut(id) {
            t.last_accessed = crate::paths::now();
        }
        let active = self.active_mut();
        active.selected = Some(id);
        active.reveal(&|i| i.is_tab(id));
        true
    }

    /// Drop a tab onto the row (or tile) at `slot`: it takes that row's
    /// place. Within one list the target shifts towards where the dragged
    /// tab came from, so dropping on the row right below moves the tab down
    /// by one; from another list it lands before the target.
    pub fn move_tab_onto(&mut self, id: TabId, space: SpaceId, slot: Slot) -> bool {
        self.move_tabs_onto(&[id], id, space, slot)
    }

    /// [`Workspace::move_tab_onto`] for several tabs moving together (a
    /// multi-selection, in order); `primary` is the one being dragged and
    /// decides which side of the target they land on.
    pub fn move_tabs_onto(&mut self, ids: &[TabId], primary: TabId, space: SpaceId, slot: Slot) -> bool {
        let after = self.location(primary).is_some_and(|from| {
            (from.space == space || from.slot.is_favorite())
                && from.slot.same_list(&slot)
                && from.slot.index() < slot.index()
        });
        let target = if after { slot.with_index(slot.index() + 1) } else { slot };
        self.move_tabs(ids, space, target)
    }

    /// Move a tab to `slot` in `space` ("before the entry now at that
    /// index"; `Favorite` slots ignore `space`). Leaving the space
    /// dissolves the tab's split pair.
    pub fn move_tab(&mut self, id: TabId, space: SpaceId, slot: Slot) -> bool {
        self.move_tabs(&[id], space, slot)
    }

    /// Move several tabs, keeping their order, to `slot` in `space`.
    pub fn move_tabs(&mut self, ids: &[TabId], space: SpaceId, slot: Slot) -> bool {
        let ids: Vec<TabId> = {
            let mut seen = HashSet::new();
            ids.iter().copied().filter(|t| self.tab(*t).is_some() && seen.insert(*t)).collect()
        };
        if ids.is_empty() || self.space(space).is_none() {
            return false;
        }
        if let Slot::Folder(f, _) = slot
            && self.space(space).and_then(|s| s.folder(f)).is_none()
        {
            return false;
        }
        if slot.is_favorite() {
            let incoming = ids.iter().filter(|t| !self.is_favorite(**t)).count();
            if self.favorites.len() + incoming > MAX_FAVORITES {
                return false;
            }
        }
        let target_space = if slot.is_favorite() { self.active_space } else { space };
        // Where each tab comes from: its space, or `None` for a favorite.
        let origins: Vec<(TabId, Option<SpaceId>)> = ids
            .iter()
            .map(|t| {
                let from = self.location(*t);
                (*t, from.filter(|l| !l.slot.is_favorite()).map(|l| l.space))
            })
            .collect();
        let fallbacks = self.selection_fallbacks(&ids);
        let anchor = self.anchor_at(space, slot, &ids, None);
        for id in &ids {
            self.detach(*id);
        }
        self.insert_at_anchor(space, slot, anchor, ids.iter().map(|t| Item::Tab(*t)).collect());
        // A pair stays together only while its tab stays where it was:
        // in the favorites, or in the same space.
        for (tab, from_space) in origins {
            let stays = if slot.is_favorite() { from_space.is_none() } else { from_space == Some(target_space) };
            if !stays {
                for s in &mut self.spaces {
                    if slot.is_favorite() || s.id != target_space {
                        s.splits.retain(|g| !g.contains(tab));
                    }
                }
            }
            self.sync_pinned_url(tab);
        }
        self.settle_selection(fallbacks);
        true
    }

    /// Move a tab to the end of another space's loose list.
    pub fn move_to_space(&mut self, id: TabId, space: SpaceId) -> bool {
        self.move_tabs_to_space(&[id], space)
    }

    /// Move tabs (in order) to the end of a space's loose list. A favorite
    /// leaves the favorites.
    pub fn move_tabs_to_space(&mut self, ids: &[TabId], space: SpaceId) -> bool {
        self.move_tabs(ids, space, Slot::Loose(usize::MAX))
    }

    /// Pin a tab: the end of its space's pinned rows (a favorite goes to
    /// the active space's).
    pub fn pin(&mut self, id: TabId) -> bool {
        let Some(loc) = self.location(id) else {
            return false;
        };
        if loc.slot.is_pinned() {
            return true;
        }
        self.move_tab(id, loc.space, Slot::Pinned(usize::MAX))
    }

    /// Unpin a favorite or pinned tab: it goes to the top of the loose list.
    pub fn unpin(&mut self, id: TabId) -> bool {
        let Some(loc) = self.location(id) else {
            return false;
        };
        if !loc.slot.is_saved() {
            return true;
        }
        self.move_tab(id, loc.space, Slot::Loose(0))
    }

    /// Pin a loose tab, or unpin a pinned one or a favorite. Returns
    /// whether it is pinned now.
    pub fn toggle_pin(&mut self, id: TabId) -> bool {
        if self.is_saved(id) {
            self.unpin(id);
            false
        } else {
            self.pin(id);
            true
        }
    }

    /// Add a tab to the end of the favorites. False when they are full.
    pub fn add_favorite(&mut self, id: TabId) -> bool {
        if self.is_favorite(id) {
            return true;
        }
        let space = self.active_space;
        self.move_tab(id, space, Slot::Favorite(usize::MAX))
    }

    /// Ordered tabs of the active space (pinned, then loose).
    pub fn ordered(&self) -> Vec<TabId> {
        self.active().ordered()
    }

    /// The tab `delta` steps from the selection, wrapping, in the active space.
    pub fn cycle(&self, delta: isize) -> Option<TabId> {
        self.cycle_from(self.selected(), delta)
    }

    /// The tab `delta` steps from `from` (e.g. the focused split pane),
    /// wrapping, among the tabs the active space shows (favorites first).
    pub fn cycle_from(&self, from: Option<TabId>, delta: isize) -> Option<TabId> {
        let order = self.visible_in(self.active_space);
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

    /// Ctrl+1..9: 1-8 pick by position among the visible tabs (favorites
    /// first), 9 picks the last one.
    pub fn by_ordinal(&self, n: usize) -> Option<TabId> {
        let order = self.visible_in(self.active_space);
        if n == 9 {
            return order.last().copied();
        }
        order.get(n.checked_sub(1)?).copied()
    }

    // ---- Folders --------------------------------------------------------------

    /// Create a folder in `space` holding `tabs` (which leave their old
    /// slots). When the tabs all sit in one pinned list (the top level or a
    /// folder) the new folder takes the first one's place there, nested if
    /// that list is a folder with room; otherwise it goes to the end of the
    /// pinned rows.
    pub fn create_folder(
        &mut self,
        space: SpaceId,
        name: impl Into<String>,
        tabs: &[TabId],
    ) -> Option<FolderId> {
        self.space(space)?;
        let locations: Vec<Slot> = tabs
            .iter()
            .filter_map(|t| self.location(*t).filter(|l| l.space == space).map(|l| l.slot))
            .collect();
        let shared = locations
            .first()
            .and_then(|first| first.pinned_list())
            .filter(|list| locations.iter().all(|l| l.pinned_list() == Some(*list)));
        let (parent, index) = match shared {
            Some(parent) => {
                let fits = parent
                    .and_then(|f| self.space(space)?.folder_depth(f))
                    .is_none_or(|depth| depth < MAX_FOLDER_DEPTH);
                let first = locations.iter().map(|l| l.index()).min().unwrap_or(usize::MAX);
                if fits { (parent, first) } else { (None, usize::MAX) }
            }
            None => (None, usize::MAX),
        };
        self.create_folder_at(space, parent, index, name, tabs)
    }

    /// Create a folder at `index` of `parent`'s items (the pinned top level
    /// if `None`) holding `tabs`.
    pub fn create_folder_at(
        &mut self,
        space: SpaceId,
        parent: Option<FolderId>,
        index: usize,
        name: impl Into<String>,
        tabs: &[TabId],
    ) -> Option<FolderId> {
        if let Some(p) = parent {
            let depth = self.space(space)?.folder_depth(p)?;
            if depth >= MAX_FOLDER_DEPTH {
                return None;
            }
        }
        let folder = Folder::new(name);
        let fid = folder.id;
        let slot = match parent {
            None => Slot::Pinned(index),
            Some(p) => Slot::Folder(p, index),
        };
        let anchor = self.anchor_at(space, slot, tabs, None);
        self.insert_at_anchor(space, slot, anchor, vec![Item::Folder(folder)]);
        self.move_tabs(tabs, space, Slot::Folder(fid, 0));
        Some(fid)
    }

    /// Delete a folder; every tab in it (subfolders too) becomes a loose
    /// tab at the top of the list.
    pub fn delete_folder(&mut self, folder: FolderId) -> bool {
        let Some((space_id, f)) = self.find_folder(folder) else {
            return false;
        };
        let tabs = f.tabs();
        let Some(space) = self.space_mut(space_id) else {
            return false;
        };
        let Some((parent, i)) = find_folder_slot(&space.pinned, folder, None) else {
            return false;
        };
        if let Some(items) = space.items_mut(parent) {
            items.remove(i);
        }
        space.loose.splice(0..0, tabs.iter().copied());
        for t in tabs {
            self.sync_pinned_url(t);
        }
        true
    }

    /// Dissolve a folder: its tabs and subfolders take its place in the
    /// folder (or pinned rows) around it, still pinned.
    pub fn ungroup_folder(&mut self, folder: FolderId) -> bool {
        let Some((space_id, _)) = self.find_folder(folder) else {
            return false;
        };
        let Some(space) = self.space_mut(space_id) else {
            return false;
        };
        let Some((parent, i)) = find_folder_slot(&space.pinned, folder, None) else {
            return false;
        };
        let Some(items) = space.items_mut(parent) else { return false };
        let Item::Folder(removed) = items.remove(i) else { return false };
        items.splice(i..i, removed.items);
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

    /// Move a folder (with everything in it) to `slot`, which must be in
    /// some space's pinned section: before the entry at that index of the
    /// top level (`Pinned`) or of another folder (`Folder`). Refused when
    /// it would land inside itself or nest deeper than
    /// [`MAX_FOLDER_DEPTH`]. Moving to another space takes its tabs along.
    pub fn move_folder(&mut self, folder: FolderId, space: SpaceId, slot: Slot) -> bool {
        let Some(parent) = slot.pinned_list() else { return false };
        let Some((from_space, f)) = self.find_folder(folder) else { return false };
        let height = f.height();
        let tabs = f.tabs();
        let Some(target) = self.space(space) else { return false };
        let depth_there = match parent {
            None => 0,
            Some(p) => {
                if p == folder || find_folder(&f.items, p).is_some() {
                    return false;
                }
                match target.folder_depth(p) {
                    Some(d) => d,
                    None => return false,
                }
            }
        };
        if depth_there + height > MAX_FOLDER_DEPTH {
            return false;
        }
        let fallbacks = if from_space != space { self.selection_fallbacks(&tabs) } else { Vec::new() };
        let anchor = self.anchor_at(space, slot, &[], Some(folder));
        let Some(source) = self.space_mut(from_space) else { return false };
        let Some((old_parent, i)) = find_folder_slot(&source.pinned, folder, None) else {
            return false;
        };
        let Some(items) = source.items_mut(old_parent) else { return false };
        let item = items.remove(i);
        if from_space != space {
            source.splits.retain(|g| !tabs.iter().any(|t| g.contains(*t)));
        }
        self.insert_at_anchor(space, slot, anchor, vec![item]);
        self.settle_selection(fallbacks);
        true
    }

    // ---- Spaces ---------------------------------------------------------------

    /// Add a space with the next unused symbol and colour.
    pub fn create_space(&mut self, name: impl Into<String>) -> SpaceId {
        let mut space = Space::new(name);
        space.symbol = self.next_space_symbol().to_string();
        space.theme = space_theme_for_colors(self.next_space_colors());
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

    /// The first colour pair in [`NEW_SPACE_COLORS`] no space is themed
    /// with yet; once all are taken they repeat in order.
    pub fn next_space_colors(&self) -> (u32, u32) {
        let used = |(c, _): (u32, u32)| {
            let rgb = Rgba::hex(c).to_hex_string();
            self.spaces
                .iter()
                .any(|s| s.theme.primary_dot().is_some_and(|d| d.rgb.with_alpha(1.0).to_hex_string() == rgb))
        };
        NEW_SPACE_COLORS
            .iter()
            .copied()
            .find(|c| !used(*c))
            .unwrap_or(NEW_SPACE_COLORS[self.spaces.len() % NEW_SPACE_COLORS.len()])
    }

    /// Default name for the next new space: "Space N".
    pub fn next_space_name(&self) -> String {
        format!("Space {}", self.spaces.len() + 1)
    }

    /// Delete a space and its tabs, returning everything needed to undo it.
    /// Favorites stay. Refuses to delete the last space. Falls back to the
    /// previous space if it was active.
    pub fn delete_space(&mut self, id: SpaceId) -> Option<DeletedSpace> {
        if self.spaces.len() <= 1 {
            return None;
        }
        let index = self.spaces.iter().position(|s| s.id == id)?;
        let space = self.spaces.remove(index);
        let doomed: HashSet<TabId> = space.ordered().into_iter().collect();
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
        let restored: HashSet<TabId> = tabs
            .iter()
            .map(|t| t.id)
            .filter(|id| self.tab(*id).is_none())
            .collect();
        self.tabs.extend(tabs.into_iter().filter(|t| restored.contains(&t.id)));
        self.reindex();
        space.retain_tabs(&mut |t: &TabId| restored.contains(t));
        let favorites: HashSet<TabId> = self.favorites.iter().copied().collect();
        let shown = |t: &TabId| restored.contains(t) || favorites.contains(t);
        space.splits.retain(|g| g.tabs.iter().all(shown));
        if space.selected.is_some_and(|s| !shown(&s)) {
            space.selected = None;
        }
        let id = space.id;
        self.spaces.insert(index.min(self.spaces.len()), space);
        if was_active {
            self.active_space = id;
        }
        id
    }

    /// Move a space in the switcher to before the space now at `index`
    /// (past the end: last).
    pub fn move_space(&mut self, id: SpaceId, index: usize) -> bool {
        let Some(from) = self.spaces.iter().position(|s| s.id == id) else {
            return false;
        };
        let anchor = self.spaces.iter().skip(index).find(|s| s.id != id).map(|s| s.id);
        let space = self.spaces.remove(from);
        let at = anchor
            .and_then(|a| self.spaces.iter().position(|s| s.id == a))
            .unwrap_or(self.spaces.len());
        self.spaces.insert(at, space);
        true
    }

    /// The space `delta` steps from the active one in switcher order,
    /// wrapping around (as Zen does). `None` with a single space.
    pub fn space_neighbor(&self, delta: isize) -> Option<SpaceId> {
        let n = self.spaces.len() as isize;
        if n < 2 {
            return None;
        }
        let current = self.spaces.iter().position(|s| s.id == self.active_space)? as isize;
        let next = (current + delta).rem_euclid(n);
        Some(self.spaces[next as usize].id)
    }

    // ---- Split view -----------------------------------------------------------

    /// The split pair `tab` belongs to, if any.
    pub fn split_of(&self, tab: TabId) -> Option<&SplitGroup> {
        self.space(self.space_of(tab)?)?.split_of(tab)
    }

    /// Pair `left` and `right` into a split view. Both must be different
    /// tabs shown in the same space; any pair either was in is dissolved.
    /// When both sit in the same loose list or folder, `right` moves next
    /// to `left` so the pair shows as one row; a tab is never moved into or
    /// out of a folder, the pinned rows, or the favorites by pairing it.
    pub fn create_split(&mut self, left: TabId, right: TabId) -> bool {
        let (Some(l), Some(r)) = (self.location(left), self.location(right)) else {
            return false;
        };
        if left == right || l.space != r.space {
            return false;
        }
        if matches!(l.slot, Slot::Loose(_) | Slot::Folder(..)) && l.slot.same_list(&r.slot) {
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

/// Give folders whose id was already seen (a hand-edited or corrupt
/// session) a fresh one, so lookups by id stay unambiguous.
fn fresh_duplicate_folder_ids(items: &mut [Item], seen: &mut HashSet<FolderId>) {
    for item in items {
        if let Item::Folder(f) = item {
            if !seen.insert(f.id) {
                f.id = Uuid::new_v4();
                seen.insert(f.id);
            }
            fresh_duplicate_folder_ids(&mut f.items, seen);
        }
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

    fn folder_tabs(ws: &Workspace, f: FolderId) -> Vec<TabId> {
        ws.find_folder(f).unwrap().1.tabs()
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
        // Tabs in folders are pinned: they keep their address.
        assert!(ws.is_pinned(ids[0]));
        assert_eq!(ws.tab(ids[0]).unwrap().pinned_url.as_deref(), Some("https://t0.test"));
        ws.update_folder(f, |f| f.is_expanded = false);
        assert_eq!(ws.active().visible(), vec![ids[1]]);
        assert!(ws.delete_folder(f));
        assert_eq!(ws.active().loose, vec![ids[0], ids[2], ids[1]]);
        assert_eq!(ws.tab(ids[0]).unwrap().pinned_url, None);
    }

    #[test]
    fn pinning_leaves_folder() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let f = ws.create_folder(space, "F", &[ids[0]]).unwrap();
        ws.pin(ids[0]);
        // Already pinned (folders are in the pinned section): it stays put.
        assert_eq!(folder_tabs(&ws, f), vec![ids[0]]);
        ws.move_tab(ids[0], space, Slot::Pinned(0));
        assert!(folder_tabs(&ws, f).is_empty());
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
        // Order: [t1 (hidden in folder), t0, t2].
        assert_eq!(ws.neighbor(ids[1]), Some(ids[0]));
        ws.active_mut().selected = Some(ids[1]);
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
    fn selecting_a_tab_opens_its_folders() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let outer = ws.create_folder(space, "Outer", &[ids[0]]).unwrap();
        let inner = ws.create_folder(space, "Inner", &[ids[0]]).unwrap();
        assert_eq!(ws.active().folder_depth(inner), Some(2));
        ws.update_folder(outer, |f| f.is_expanded = false);
        ws.update_folder(inner, |f| f.is_expanded = false);
        assert_eq!(ws.by_ordinal(1), Some(ids[1]), "hidden tabs aren't counted");
        ws.select(ids[0]);
        assert!(ws.active().folder(outer).unwrap().is_expanded);
        assert!(ws.active().folder(inner).unwrap().is_expanded);
    }

    #[test]
    fn new_tabs_go_to_top_of_loose_list() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[0]]).unwrap();
        ws.select(ids[0]);
        let n = ws.add_new_tab(TabRecord::new("https://new.test"));
        assert_eq!(ws.active().loose, vec![n, ids[1]]);
        assert_eq!(folder_tabs(&ws, f), vec![ids[0]]);
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
    fn child_tabs_of_saved_tabs_stay_out_of_collections() {
        let (mut ws, ids) = ws_with(3);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[0]]).unwrap();
        ws.pin(ids[1]);
        ws.add_favorite(ids[2]);
        let a = ws.add_child_tab(TabRecord::new("https://a.test"), ids[0], None);
        assert_eq!(folder_tabs(&ws, f), vec![ids[0]]);
        assert_eq!(ws.active().loose, vec![a]);
        let b = ws.add_child_tab(TabRecord::new("https://b.test"), ids[0], Some(a));
        assert_eq!(ws.active().loose, vec![a, b]);
        let c = ws.add_child_tab(TabRecord::new("https://c.test"), ids[1], None);
        assert_eq!(ws.active().ordered()[..2], [ids[0], ids[1]]);
        assert_eq!(ws.active().loose, vec![c, a, b]);
        let d = ws.add_child_tab(TabRecord::new("https://d.test"), ids[2], None);
        assert_eq!(ws.favorites, vec![ids[2]]);
        assert_eq!(ws.active().loose, vec![d, c, a, b]);
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
    fn drop_onto_favorite_tile_and_favorites_end() {
        let (mut ws, ids) = ws_with(4);
        let space = ws.active_space;
        for id in &ids[..3] {
            assert!(ws.add_favorite(*id));
        }
        ws.move_tab_onto(ids[0], space, Slot::Favorite(1));
        assert_eq!(ws.favorites, vec![ids[1], ids[0], ids[2]]);
        // A loose tab dropped on a tile lands before it.
        ws.move_tab_onto(ids[3], space, Slot::Favorite(0));
        assert_eq!(ws.favorites, vec![ids[3], ids[1], ids[0], ids[2]]);
        // Dropping past the last tile adds at the end.
        ws.move_tab(ids[3], space, Slot::Favorite(usize::MAX));
        assert_eq!(ws.favorites, vec![ids[1], ids[0], ids[2], ids[3]]);
        assert!(ws.active().ordered().is_empty());
    }

    #[test]
    fn favorites_are_shared_by_every_space() {
        let (mut ws, ids) = ws_with(2);
        let personal = ws.active_space;
        let work = ws.create_space("Work");
        ws.add_favorite(ids[0]);
        assert!(ws.is_favorite(ids[0]) && ws.is_saved(ids[0]) && !ws.is_pinned(ids[0]));
        assert!(!ws.active().contains(ids[0]));
        // Selecting a favorite keeps the space you're in.
        ws.switch_space(work);
        ws.select(ids[0]);
        assert_eq!(ws.active_space, work);
        assert_eq!(ws.selected(), Some(ids[0]));
        assert_eq!(ws.visible_in(work), vec![ids[0]]);
        assert_eq!(ws.visible_in(personal), vec![ids[0], ids[1]]);
        // Deleting a space never takes the favorites with it.
        ws.delete_space(work);
        assert!(ws.tab(ids[0]).is_some());
        assert_eq!(ws.favorites, vec![ids[0]]);
    }

    #[test]
    fn favorites_have_a_limit() {
        let (mut ws, ids) = ws_with(MAX_FAVORITES + 1);
        for id in &ids[..MAX_FAVORITES] {
            assert!(ws.add_favorite(*id));
        }
        assert!(!ws.add_favorite(ids[MAX_FAVORITES]));
        assert_eq!(ws.active().loose, vec![ids[MAX_FAVORITES]]);
        // Reordering inside a full grid still works.
        let space = ws.active_space;
        assert!(ws.move_tab(ids[0], space, Slot::Favorite(3)));
    }

    #[test]
    fn closing_a_selected_favorite_fixes_every_space() {
        let (mut ws, ids) = ws_with(2);
        let personal = ws.active_space;
        let work = ws.create_space("Work");
        let w = ws.add_tab(TabRecord::new("https://w.test"), Some(work), Slot::Loose(0));
        ws.add_favorite(ids[0]);
        ws.select(ids[0]);
        ws.switch_space(work);
        ws.select(ids[0]);
        ws.remove_tab(ids[0]);
        assert_eq!(ws.space(work).unwrap().selected, Some(w));
        assert_eq!(ws.space(personal).unwrap().selected, Some(ids[1]));
    }

    #[test]
    fn moving_a_favorite_to_another_space() {
        let (mut ws, ids) = ws_with(2);
        let work = ws.create_space("Work");
        ws.add_favorite(ids[0]);
        ws.select(ids[0]);
        ws.move_to_space(ids[0], work);
        assert!(!ws.is_favorite(ids[0]));
        assert_eq!(ws.space_of(ids[0]), Some(work));
        assert_eq!(ws.tab(ids[0]).unwrap().pinned_url, None);
        assert_eq!(ws.selected(), Some(ids[1]));
    }

    #[test]
    fn pinned_tabs_remember_their_address() {
        let (mut ws, ids) = ws_with(2);
        ws.pin(ids[0]);
        assert_eq!(ws.tab(ids[0]).unwrap().pinned_url.as_deref(), Some("https://t0.test"));
        assert!(!ws.tab(ids[0]).unwrap().left_pinned_url());
        // Same page with a fragment or trailing slash isn't "away".
        ws.set_url(ids[0], "https://t0.test/#top");
        assert!(!ws.tab(ids[0]).unwrap().left_pinned_url());
        ws.set_url(ids[0], "https://t0.test/inbox");
        assert!(ws.tab(ids[0]).unwrap().left_pinned_url());
        assert_eq!(ws.reset_to_pinned(ids[0]).as_deref(), Some("https://t0.test"));
        assert_eq!(ws.tab(ids[0]).unwrap().url, "https://t0.test");
        assert_eq!(ws.reset_to_pinned(ids[0]), None, "already home");
        // Replacing makes the current page the pinned one.
        ws.set_url(ids[0], "https://t0.test/inbox");
        assert!(ws.replace_pinned_url(ids[0]));
        assert!(!ws.tab(ids[0]).unwrap().left_pinned_url());
        // Loose tabs have no pinned address.
        ws.set_url(ids[1], "https://elsewhere.test");
        assert_eq!(ws.tab(ids[1]).unwrap().pinned_url, None);
        assert!(!ws.replace_pinned_url(ids[1]));
        ws.unpin(ids[0]);
        assert_eq!(ws.tab(ids[0]).unwrap().pinned_url, None);
    }

    #[test]
    fn a_tab_pinned_while_blank_adopts_its_first_real_address() {
        let (mut ws, _) = ws_with(0);
        let blank = ws.add_new_tab(TabRecord::new("mori://newtab"));
        ws.pin(blank);
        assert_eq!(ws.tab(blank).unwrap().pinned_url, None);
        ws.set_url(blank, "https://mail.test/");
        assert_eq!(ws.tab(blank).unwrap().pinned_url.as_deref(), Some("https://mail.test/"));
        ws.set_url(blank, "https://mail.test/thread/1");
        assert_eq!(ws.tab(blank).unwrap().pinned_url.as_deref(), Some("https://mail.test/"));
    }

    #[test]
    fn toggling_a_pin_and_unpinning_favorites() {
        let (mut ws, ids) = ws_with(2);
        assert!(ws.toggle_pin(ids[1]));
        assert_eq!(ws.active().pinned, vec![Item::Tab(ids[1])]);
        assert!(!ws.toggle_pin(ids[1]));
        assert_eq!(ws.active().loose, vec![ids[1], ids[0]]);
        ws.add_favorite(ids[0]);
        assert!(!ws.toggle_pin(ids[0]), "a favorite unpins to Today");
        assert_eq!(ws.active().loose, vec![ids[0], ids[1]]);
        // Pinning a favorite moves it into the space's pinned rows.
        ws.add_favorite(ids[0]);
        ws.pin(ids[0]);
        assert!(ws.favorites.is_empty() && ws.is_pinned(ids[0]));
    }

    #[test]
    fn nested_folders_order_and_depth_limit() {
        let (mut ws, ids) = ws_with(5);
        let space = ws.active_space;
        let a = ws.create_folder(space, "A", &[ids[0], ids[1]]).unwrap();
        // A folder made from tabs inside a folder nests there, in their place.
        let b = ws.create_folder(space, "B", &[ids[1]]).unwrap();
        assert_eq!(ws.active().folder_depth(b), Some(2));
        assert_eq!(ws.active().folder(a).unwrap().items[1], Item::Folder(ws.active().folder(b).unwrap().clone()));
        let c = ws.create_folder(space, "C", &[ids[1]]).unwrap();
        assert_eq!(ws.active().folder_depth(c), Some(MAX_FOLDER_DEPTH));
        let trail: Vec<&str> = ws.active().folder_trail(c).iter().map(|f| f.name.as_str()).collect();
        assert_eq!(trail, ["A", "B", "C"]);
        // No room for a fourth level: the folder goes to the top level.
        let d = ws.create_folder(space, "D", &[ids[1]]).unwrap();
        assert_eq!(ws.active().folder_depth(d), Some(1));
        assert!(ws.create_folder_at(space, Some(c), 0, "E", &[]).is_none());
        assert_eq!(ws.ordered(), vec![ids[0], ids[1], ids[2], ids[3], ids[4]]);
        // Moving C (height 1) into D is fine; D (now height 2) can't go
        // into B (depth 2): that would make four levels.
        assert!(ws.move_folder(c, space, Slot::Folder(d, 0)));
        assert!(!ws.move_folder(d, space, Slot::Folder(b, 0)));
        // A folder can't go inside itself or its own subfolder.
        assert!(!ws.move_folder(a, space, Slot::Folder(a, 0)));
        assert!(!ws.move_folder(a, space, Slot::Folder(b, 0)));
        // Ungrouping keeps the contents pinned, in the folder's place.
        assert!(ws.ungroup_folder(a));
        assert!(matches!(ws.active().pinned[0], Item::Tab(t) if t == ids[0]));
        assert!(ws.active().pinned[1].is_folder(b));
    }

    #[test]
    fn folders_reorder_and_move_between_spaces() {
        let (mut ws, ids) = ws_with(4);
        let space = ws.active_space;
        let a = ws.create_folder(space, "A", &[ids[0]]).unwrap();
        let b = ws.create_folder(space, "B", &[ids[1]]).unwrap();
        ws.pin(ids[2]);
        // [A, B, t2] → move A before t2 → [B, A, t2].
        assert!(ws.move_folder(a, space, Slot::Pinned(2)));
        let order: Vec<bool> = ws.active().pinned.iter().map(|i| i.is_folder(b)).collect();
        assert_eq!(order, vec![true, false, false]);
        assert!(ws.active().pinned[1].is_folder(a));
        // Into another space, tabs and all; selection falls back here.
        let work = ws.create_space("Work");
        ws.select(ids[0]);
        assert!(ws.move_folder(a, work, Slot::Pinned(0)));
        assert_eq!(ws.space_of(ids[0]), Some(work));
        assert_eq!(ws.space(space).unwrap().selected, Some(ids[2]));
        // Folders can't go to the loose list or the favorites.
        assert!(!ws.move_folder(b, space, Slot::Loose(0)));
        assert!(!ws.move_folder(b, space, Slot::Favorite(0)));
    }

    #[test]
    fn moving_a_multi_selection_keeps_its_order() {
        let (mut ws, ids) = ws_with(5);
        let space = ws.active_space;
        // Drag [t3, t1] onto t0's row: they land before it, in sidebar order.
        let group = ws.in_sidebar_order(&[ids[3], ids[1]]);
        assert_eq!(group, vec![ids[1], ids[3]]);
        ws.move_tabs_onto(&group, ids[3], space, Slot::Loose(0));
        assert_eq!(ws.ordered(), vec![ids[1], ids[3], ids[0], ids[2], ids[4]]);
        // Onto a row below the dragged tab: after it.
        ws.move_tabs_onto(&[ids[1], ids[3]], ids[1], space, Slot::Loose(3));
        assert_eq!(ws.ordered(), vec![ids[0], ids[2], ids[1], ids[3], ids[4]]);
        // Into a folder, and pinned together.
        let f = ws.create_folder(space, "F", &[]).unwrap();
        ws.move_tabs(&[ids[0], ids[4]], space, Slot::Folder(f, 0));
        assert_eq!(folder_tabs(&ws, f), vec![ids[0], ids[4]]);
        assert!(ws.is_pinned(ids[4]));
        // To another space.
        let work = ws.create_space("Work");
        ws.move_tabs_to_space(&[ids[2], ids[3]], work);
        assert_eq!(ws.space(work).unwrap().loose, vec![ids[2], ids[3]]);
    }

    #[test]
    fn closing_a_selection_falls_back_outside_it() {
        let (mut ws, ids) = ws_with(4);
        ws.pin(ids[0]);
        // Order: [t0 pinned, t1, t2, t3]; closing t0..t2 from t0 lands on t3.
        assert_eq!(ws.neighbor_skipping(ids[0], &ids[..3]), Some(ids[3]));
        assert_eq!(ws.neighbor_skipping(ids[3], &ids[1..]), Some(ids[0]));
        assert_eq!(ws.neighbor_skipping(ids[1], &ids), None);
    }

    #[test]
    fn shift_click_range_follows_the_sidebar() {
        let (mut ws, ids) = ws_with(4);
        ws.add_favorite(ids[3]);
        assert_eq!(ws.visible_range(ids[1], ids[3]), vec![ids[3], ids[0], ids[1]]);
        assert_eq!(ws.visible_range(ids[2], ids[2]), vec![ids[2]]);
        assert!(ws.visible_range(ids[0], Uuid::new_v4()).is_empty());
    }

    #[test]
    fn spaces_reorder_and_step() {
        let (mut ws, _) = ws_with(0);
        let a = ws.active_space;
        let b = ws.create_space("B");
        let c = ws.create_space("C");
        assert_eq!(ws.space_neighbor(1), Some(b));
        assert_eq!(ws.space_neighbor(-1), Some(c), "wraps around");
        ws.move_space(a, 3);
        assert_eq!(ws.spaces.iter().map(|s| s.id).collect::<Vec<_>>(), vec![b, c, a]);
        ws.move_space(c, 0);
        assert_eq!(ws.spaces.iter().map(|s| s.id).collect::<Vec<_>>(), vec![c, b, a]);
        ws.move_space(c, 1);
        assert_eq!(ws.spaces.iter().map(|s| s.id).collect::<Vec<_>>(), vec![c, b, a], "own place");
        ws.delete_space(b);
        ws.delete_space(c);
        assert_eq!(ws.space_neighbor(1), None);
    }

    #[test]
    fn new_spaces_get_distinct_colors() {
        let (mut ws, _) = ws_with(0);
        assert!(ws.active().theme.is_none(), "the first space stays plain");
        let a = ws.create_space("A");
        let b = ws.create_space("B");
        let primary = |ws: &Workspace, id| ws.space(id).unwrap().theme.primary_dot().unwrap().rgb.to_hex_string();
        assert_eq!(primary(&ws, a), Rgba::hex(NEW_SPACE_COLORS[0].0).to_hex_string());
        assert_eq!(primary(&ws, b), Rgba::hex(NEW_SPACE_COLORS[1].0).to_hex_string());
        assert_eq!(ws.space(a).unwrap().theme.dots.len(), 2);
        // A freed colour is handed out again first.
        ws.delete_space(a);
        let c = ws.create_space("C");
        assert_eq!(primary(&ws, c), Rgba::hex(NEW_SPACE_COLORS[0].0).to_hex_string());
    }

    /// New spaces keep dark chrome with light text, so they all read alike
    /// whatever colour they get.
    #[test]
    fn new_space_colors_keep_the_chrome_dark() {
        for pair in NEW_SPACE_COLORS {
            assert!(crate::theme::effective_dark(&space_theme_for_colors(pair), false), "{pair:x?}");
        }
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
    fn pairing_never_moves_a_tab_into_or_out_of_a_folder() {
        let (mut ws, ids) = ws_with(3);
        let space = ws.active_space;
        let f = ws.create_folder(space, "Work", &[ids[0]]).unwrap();
        assert!(ws.create_split(ids[0], ids[2]));
        assert_eq!(folder_tabs(&ws, f), vec![ids[0]]);
        assert_eq!(ws.active().loose, vec![ids[1], ids[2]]);
        assert!(ws.create_split(ids[2], ids[0]));
        assert_eq!(folder_tabs(&ws, f), vec![ids[0]]);
        assert_eq!(ws.split_of(ids[0]).unwrap().tabs, [ids[2], ids[0]]);
    }

    #[test]
    fn a_favorite_can_be_split_with_a_space_tab() {
        let (mut ws, ids) = ws_with(2);
        ws.add_favorite(ids[0]);
        assert!(ws.create_split(ids[0], ids[1]));
        assert_eq!(ws.favorites, vec![ids[0]]);
        let json = serde_json::to_string(&ws).unwrap();
        let mut back: Workspace = serde_json::from_str(&json).unwrap();
        back.repair();
        assert_eq!(back.split_of(ids[1]).map(|g| g.tabs), Some([ids[0], ids[1]]));
        // The favorite leaving for good ends the pair everywhere.
        ws.remove_tab(ids[0]);
        assert!(ws.active().splits.is_empty());
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
        assert_eq!(ws.version, 3);
        ws.repair();
        assert_eq!(ws.version, SESSION_VERSION);
        assert_eq!(ws.tabs[0].scroll_y, 0.0);
        assert!(ws.closed.is_empty());
        assert!(ws.active().splits.is_empty());
        assert!(ws.favorites.is_empty());
    }

    /// A version 3 session: a pinned grid per space plus a separate flat
    /// folder list. Pins become that space's pinned rows, folders follow
    /// them in the pinned section, and every saved tab gets its address as
    /// its pinned URL. Nothing is lost.
    #[test]
    fn version_3_sessions_migrate_to_pinned_rows() {
        let json = r#"{
            "version": 3,
            "tabs": [
                {"id":"00000000-0000-4000-8000-000000000001","url":"https://mail.test/inbox","title":"Mail"},
                {"id":"00000000-0000-4000-8000-000000000002","url":"https://cal.test","title":"Calendar"},
                {"id":"00000000-0000-4000-8000-000000000003","url":"https://docs.test/a"},
                {"id":"00000000-0000-4000-8000-000000000004","url":"https://news.test"},
                {"id":"00000000-0000-4000-8000-000000000005","url":"https://code.test"}
            ],
            "spaces": [
                {
                    "id":"00000000-0000-4000-8000-0000000000a1","name":"Personal",
                    "pinned":["00000000-0000-4000-8000-000000000001","00000000-0000-4000-8000-000000000002"],
                    "folders":[{"id":"00000000-0000-4000-8000-0000000000f1","name":"Docs","is_expanded":false,
                                "tab_ids":["00000000-0000-4000-8000-000000000003"]}],
                    "loose":["00000000-0000-4000-8000-000000000004"],
                    "selected":"00000000-0000-4000-8000-000000000003"
                },
                {
                    "id":"00000000-0000-4000-8000-0000000000a2","name":"Work","symbol":"briefcase",
                    "pinned":["00000000-0000-4000-8000-000000000005"]
                }
            ],
            "active_space":"00000000-0000-4000-8000-0000000000a1"
        }"#;
        let id = |n: u8| Uuid::parse_str(&format!("00000000-0000-4000-8000-0000000000{n:02x}")).unwrap();
        let mut ws: Workspace = serde_json::from_str(json).unwrap();
        ws.repair();
        assert_eq!(ws.tabs.len(), 5);
        assert!(ws.favorites.is_empty());
        let personal = ws.space(id(0xa1)).unwrap();
        assert_eq!(personal.pinned.len(), 3);
        assert_eq!(personal.pinned[0], Item::Tab(id(1)));
        assert_eq!(personal.pinned[1], Item::Tab(id(2)));
        let docs = personal.folder(id(0xf1)).unwrap();
        assert_eq!(docs.name, "Docs");
        assert!(!docs.is_expanded);
        assert_eq!(docs.items, vec![Item::Tab(id(3))]);
        assert_eq!(personal.loose, vec![id(4)]);
        assert_eq!(personal.selected, Some(id(3)));
        assert_eq!(ws.space(id(0xa2)).unwrap().pinned, vec![Item::Tab(id(5))]);
        for (tab, url) in [(1, "https://mail.test/inbox"), (2, "https://cal.test"), (3, "https://docs.test/a"), (5, "https://code.test")] {
            assert!(ws.is_pinned(id(tab)));
            assert_eq!(ws.tab(id(tab)).unwrap().pinned_url.as_deref(), Some(url));
        }
        assert_eq!(ws.tab(id(4)).unwrap().pinned_url, None);
        // And the migrated session saves in the new shape and loads back the same.
        let saved = serde_json::to_value(&ws).unwrap();
        assert_eq!(saved["version"], SESSION_VERSION);
        assert!(saved["spaces"][0].get("folders").is_none());
        assert_eq!(saved["spaces"][0]["pinned"][2]["items"][0], "00000000-0000-4000-8000-000000000003");
        let mut back: Workspace = serde_json::from_value(saved).unwrap();
        back.repair();
        assert_eq!(back, ws);
    }

    #[test]
    fn repair_gives_duplicate_folders_their_own_ids() {
        let (mut ws, ids) = ws_with(2);
        let space = ws.active_space;
        let f = ws.create_folder(space, "A", &[ids[0]]).unwrap();
        let copy = ws.active().folder(f).unwrap().clone();
        let mut copy = copy;
        copy.items = vec![Item::Tab(ids[1])];
        ws.active_mut().pinned.push(Item::Folder(copy));
        ws.active_mut().loose.clear();
        ws.repair();
        let folders = ws.active().all_folders();
        assert_eq!(folders.len(), 2);
        assert_ne!(folders[0].0.id, folders[1].0.id);
        assert_eq!(ws.ordered(), ids);
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
    fn emoji_symbols_are_told_apart_from_icons() {
        assert!(is_emoji_symbol("🚀"));
        assert!(!is_emoji_symbol("briefcase"));
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
        // Favorites come first, as they sit at the top of the sidebar.
        ws.add_favorite(ids[2]);
        assert_eq!(ws.by_ordinal(1), Some(ids[2]));
        assert_eq!(ws.cycle_from(Some(ids[2]), 1), Some(ids[0]));
    }

    #[test]
    fn repair_drops_dangling_and_adopts_orphans() {
        let (mut ws, ids) = ws_with(2);
        ws.active_mut().loose.retain(|t| *t != ids[1]);
        ws.active_mut().pinned.push(Item::Tab(Uuid::new_v4()));
        ws.active_mut().loose.push(ids[0]); // duplicate
        ws.favorites.push(Uuid::new_v4());
        ws.repair();
        assert_eq!(ws.ordered(), vec![ids[0], ids[1]]);
        assert!(ws.favorites.is_empty());
    }

    #[test]
    fn serde_roundtrip() {
        let (mut ws, ids) = ws_with(3);
        ws.select(ids[0]);
        ws.add_favorite(ids[2]);
        let json = serde_json::to_string(&ws).unwrap();
        let mut back: Workspace = serde_json::from_str(&json).unwrap();
        back.repair();
        assert_eq!(back.ordered(), &ids[..2]);
        assert_eq!(back.favorites, vec![ids[2]]);
        assert_eq!(back.tab(ids[1]).unwrap().url, "https://t1.test");
        assert_eq!(back.selected(), Some(ids[0]));
    }
}
