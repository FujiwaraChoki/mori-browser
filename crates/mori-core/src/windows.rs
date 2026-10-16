//! Several browser windows over one workspace.
//!
//! Every window shows the same spaces, folders and tabs, but each has its
//! own selection: which space it shows and which tab is selected in each
//! space (Arc). The workspace stores one selection (`active_space` and each
//! space's `selected`); a window keeps its own [`WindowSelection`] and lays
//! it over the shared tabs with [`adopt_workspace`].
//!
//! `windows.json` remembers the open windows (their selection and size) so
//! a restart brings all of them back.
//!
//! Besides normal windows there are private windows and Little Mori windows
//! (a link opened from another app); [`WindowKind::shares_with`] says what
//! two windows keep in step.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    paths,
    theme::{GradientDot, GradientTheme, Rgba, SchemeOverride},
    workspace::{SpaceId, TabId, Workspace},
};

pub const WINDOWS_FILE: &str = "windows.json";

/// What a window is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    Normal,
    /// Browses without writing anything to disk.
    Private,
    /// Little Mori: one page opened from another app, in a small window.
    Little,
}

/// What two windows keep in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sharing {
    /// Spaces, folders and tabs.
    pub workspace: bool,
    /// Settings, bookmarks, Air Traffic Control rules and Boosts.
    pub preferences: bool,
    /// History, archive, downloads, site zoom and site permissions.
    pub browsing_data: bool,
}

impl WindowKind {
    /// Normal windows share everything. A Little Mori window has its own
    /// one-page workspace but browses like a normal window. A private window
    /// has its own workspace and keeps what it browses to itself; it only
    /// follows (and changes) the preferences.
    pub fn shares_with(self, other: WindowKind) -> Sharing {
        use WindowKind::*;
        Sharing {
            workspace: self == Normal && other == Normal,
            preferences: true,
            browsing_data: self != Private && other != Private,
        }
    }

    /// The window that writes the shared stores to disk among windows of
    /// `kinds` (in opening order): the first normal window, else the first
    /// Little Mori window. Private windows never write.
    pub fn writer(kinds: &[WindowKind]) -> Option<usize> {
        kinds
            .iter()
            .position(|k| *k == WindowKind::Normal)
            .or_else(|| kinds.iter().position(|k| *k == WindowKind::Little))
    }
}

/// Where a window is in the workspace.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSelection {
    pub active_space: Option<SpaceId>,
    /// The selected tab of each space that has one.
    pub selected: Vec<(SpaceId, TabId)>,
}

impl WindowSelection {
    /// The selection `ws` holds.
    pub fn of(ws: &Workspace) -> WindowSelection {
        WindowSelection {
            active_space: Some(ws.active_space),
            selected: ws
                .spaces
                .iter()
                .filter_map(|s| s.selected.map(|t| (s.id, t)))
                .collect(),
        }
    }

    /// Make this the selection of `ws`. Spaces and tabs that no longer
    /// exist are skipped: the space stays as it was, a space whose tab is
    /// gone has nothing selected.
    pub fn apply(&self, ws: &mut Workspace) {
        if let Some(space) = self.active_space.filter(|s| ws.space(*s).is_some()) {
            ws.active_space = space;
        }
        // A space may show a favorite, which belongs to no space.
        let favorites = ws.favorites.clone();
        for space in &mut ws.spaces {
            space.selected = self
                .selected
                .iter()
                .find(|(s, _)| *s == space.id)
                .map(|(_, t)| *t)
                .filter(|t| space.contains(*t) || favorites.contains(t));
        }
    }
}

/// `incoming` (another window's workspace after a change) with `local`'s
/// selection laid over it. A tab `local` had selected that went away hands
/// over to its neighbour in `local`, as if it had been closed here.
pub fn adopt_workspace(local: &Workspace, incoming: &Workspace) -> Workspace {
    let mut out = incoming.clone();
    let mut selection = WindowSelection::of(local);
    for (space, tab) in &mut selection.selected {
        let shown_in = |ws: &Workspace, t: TabId| {
            ws.is_favorite(t) || ws.space(*space).is_some_and(|s| s.contains(t))
        };
        let still_here = shown_in(&out, *tab);
        if !still_here
            && let Some(next) = local.neighbor(*tab).filter(|n| shown_in(&out, *n))
        {
            *tab = next;
        }
    }
    selection.apply(&mut out);
    out
}

/// Whether `a` and `b` hold the same tabs, spaces and folders, whatever
/// each selects.
pub fn same_content(a: &Workspace, b: &Workspace) -> bool {
    if a.tabs.len() != b.tabs.len() || a.spaces.len() != b.spaces.len() {
        return false;
    }
    let mut a = a.clone();
    WindowSelection::of(b).apply(&mut a);
    a == *b
}

/// The chrome of a private window: a deep violet wash, always dark, so it
/// can't be mistaken for a normal window.
pub fn private_theme() -> GradientTheme {
    let mut theme = GradientTheme {
        dots: [0x33255C, 0x1B1533, 0x0E0B16]
            .iter()
            .enumerate()
            .map(|(i, &c)| GradientDot::from_rgb(Rgba::hex(c), i == 0))
            .collect(),
        ..GradientTheme::none()
    };
    theme.opacity = 0.45;
    theme.texture = 0.25;
    theme.scheme_override = SchemeOverride::Dark;
    theme
}

/// A window to bring back on the next start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedWindow {
    pub selection: WindowSelection,
    /// Logical width and height.
    pub size: Option<(f32, f32)>,
}

/// The browser windows that were open, in the order they were opened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowSession {
    pub windows: Vec<SavedWindow>,
}

/// Windows smaller than this (or absurdly large) are not restored at that
/// size.
const MIN_SIZE: f32 = 320.0;
const MAX_SIZE: f32 = 16_384.0;

impl WindowSession {
    pub fn load(dir: &Path) -> WindowSession {
        let mut session: WindowSession = paths::read_json(&dir.join(WINDOWS_FILE)).unwrap_or_default();
        for w in &mut session.windows {
            w.size = w.size.filter(|(width, height)| {
                (MIN_SIZE..=MAX_SIZE).contains(width) && (MIN_SIZE..=MAX_SIZE).contains(height)
            });
        }
        session
    }

    /// The windows to open on start: the saved ones, or a single window
    /// with the workspace's own selection when none were saved.
    pub fn restore(&self, ws: &Workspace) -> Vec<SavedWindow> {
        if self.windows.is_empty() {
            return vec![SavedWindow {
                selection: WindowSelection::of(ws),
                size: None,
            }];
        }
        self.windows.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{paths::test_support::TempDir, workspace::TabRecord};

    fn two_spaces() -> (Workspace, SpaceId, SpaceId, Vec<TabId>) {
        let mut ws = Workspace::new();
        let personal = ws.active_space;
        let a = ws.add_new_tab(TabRecord::new("https://a.com"));
        let b = ws.add_new_tab(TabRecord::new("https://b.com"));
        let work = ws.create_space("Work");
        let c = ws.add_tab(TabRecord::new("https://c.com"), Some(work), crate::workspace::Slot::Loose(0));
        ws.select(a);
        (ws, personal, work, vec![a, b, c])
    }

    #[test]
    fn selection_round_trips_and_skips_missing_tabs() {
        let (mut ws, personal, work, ids) = two_spaces();
        ws.select(ids[2]);
        let sel = WindowSelection::of(&ws);
        assert_eq!(sel.active_space, Some(work));
        ws.select(ids[1]);
        sel.apply(&mut ws);
        assert_eq!(ws.active_space, work);
        assert_eq!(ws.selected(), Some(ids[2]));
        assert_eq!(ws.space(personal).unwrap().selected, Some(ids[0]));

        // A tab from another space or a vanished space is not selected.
        let bogus = WindowSelection {
            active_space: Some(uuid::Uuid::new_v4()),
            selected: vec![(personal, ids[2])],
        };
        bogus.apply(&mut ws);
        assert_eq!(ws.active_space, work);
        assert_eq!(ws.space(personal).unwrap().selected, None);
    }

    #[test]
    fn a_selected_favorite_survives_apply_and_adopt() {
        let (mut ws, personal, work, ids) = two_spaces();
        assert!(ws.add_favorite(ids[1]));
        ws.select(ids[1]);
        let sel = WindowSelection::of(&ws);
        ws.select(ids[0]);
        sel.apply(&mut ws);
        assert_eq!(ws.space(personal).unwrap().selected, Some(ids[1]));

        // Another window's change keeps this window on its favorite.
        let mut other = ws.clone();
        other.select(ids[2]);
        let adopted = adopt_workspace(&ws, &other);
        assert_eq!(adopted.space(personal).unwrap().selected, Some(ids[1]));
        assert_eq!(adopted.active_space, personal);
        assert!(adopted.space(work).is_some());
    }

    #[test]
    fn adopting_keeps_own_selection() {
        let (ws, personal, work, ids) = two_spaces();
        // Window B looks at Work while A (ws) stays on Personal.
        let mut b = ws.clone();
        b.select(ids[2]);
        // A adds a tab.
        let mut a = ws.clone();
        let d = a.add_new_tab(TabRecord::new("https://d.com"));
        a.select(d);
        let adopted = adopt_workspace(&b, &a);
        assert!(adopted.tab(d).is_some());
        assert_eq!(adopted.active_space, work);
        assert_eq!(adopted.selected(), Some(ids[2]));
        // The Personal space keeps B's own selection, not A's new tab.
        assert_eq!(adopted.space(personal).unwrap().selected, Some(ids[0]));
        assert!(same_content(&adopted, &a));
    }

    #[test]
    fn a_tab_closed_elsewhere_hands_over_to_its_neighbour() {
        let (ws, personal, _, ids) = two_spaces();
        // b.com sits above a.com (new tabs go to the top).
        let mut b = ws.clone();
        b.select(ids[1]);
        let mut a = ws.clone();
        a.close_tab(ids[1]);
        let adopted = adopt_workspace(&b, &a);
        assert_eq!(adopted.space(personal).unwrap().selected, Some(ids[0]));
        assert_eq!(adopted.selected(), Some(ids[0]));
    }

    #[test]
    fn content_comparison_ignores_selection() {
        let (ws, _, _, ids) = two_spaces();
        let mut other = ws.clone();
        // Another window looking elsewhere (select() would also stamp the
        // tab's last use, which is content).
        WindowSelection { active_space: other.space_of(ids[2]), selected: vec![] }.apply(&mut other);
        assert_ne!(other.active_space, ws.active_space);
        assert!(same_content(&ws, &other));
        other.tab_mut(ids[0]).unwrap().title = "Renamed".into();
        assert!(!same_content(&ws, &other));
    }

    #[test]
    fn session_round_trips_and_drops_bad_sizes() {
        let tmp = TempDir::new();
        let (ws, _, work, ids) = two_spaces();
        let session = WindowSession {
            windows: vec![
                SavedWindow { selection: WindowSelection::of(&ws), size: Some((1280.0, 820.0)) },
                SavedWindow {
                    selection: WindowSelection { active_space: Some(work), selected: vec![(work, ids[2])] },
                    size: Some((10.0, 5000.0)),
                },
            ],
        };
        paths::write_json_atomic(&tmp.path().join(WINDOWS_FILE), &session).unwrap();
        let loaded = WindowSession::load(tmp.path());
        assert_eq!(loaded.windows.len(), 2);
        assert_eq!(loaded.windows[0], session.windows[0]);
        assert_eq!(loaded.windows[1].size, None);
        assert_eq!(loaded.windows[1].selection, session.windows[1].selection);
    }

    #[test]
    fn what_windows_share() {
        use WindowKind::*;
        let all = Sharing { workspace: true, preferences: true, browsing_data: true };
        assert_eq!(Normal.shares_with(Normal), all);
        // Little Mori browses like a normal window but keeps its own page.
        assert_eq!(Little.shares_with(Normal), Sharing { workspace: false, ..all });
        assert_eq!(Normal.shares_with(Little), Sharing { workspace: false, ..all });
        // Private windows only follow preferences, both ways.
        let prefs = Sharing { workspace: false, preferences: true, browsing_data: false };
        for other in [Normal, Little, Private] {
            assert_eq!(Private.shares_with(other), prefs);
            assert_eq!(other.shares_with(Private), prefs);
        }
    }

    #[test]
    fn one_window_writes_and_never_a_private_one() {
        use WindowKind::*;
        assert_eq!(WindowKind::writer(&[Little, Private, Normal, Normal]), Some(2));
        assert_eq!(WindowKind::writer(&[Private, Little]), Some(1));
        assert_eq!(WindowKind::writer(&[Private, Private]), None);
        assert_eq!(WindowKind::writer(&[]), None);
    }

    #[test]
    fn private_chrome_is_dark_whatever_the_appearance() {
        let theme = private_theme();
        assert!(crate::theme::effective_dark(&theme, false));
        assert!(crate::theme::effective_dark(&theme, true));
        assert!(!theme.is_none());
        assert!(crate::theme::accent_for(&theme, true).is_some());
    }

    #[test]
    fn restoring_without_a_saved_session_opens_one_window() {
        let tmp = TempDir::new();
        let (ws, _, _, ids) = two_spaces();
        let restored = WindowSession::load(tmp.path()).restore(&ws);
        assert_eq!(restored.len(), 1);
        let mut copy = ws.clone();
        copy.select(ids[2]);
        restored[0].selection.apply(&mut copy);
        assert_eq!(copy.selected(), Some(ids[0]));
    }
}
