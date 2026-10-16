//! Ctrl+Tab: switching between tabs in most-recently-used order (Arc).
//!
//! [`RecentTabs`] remembers the order tabs were looked at; a
//! [`Switcher`] is one Ctrl+Tab session: it starts on the tab used before
//! the current one, Tab / Shift+Tab move through the list, and releasing
//! Ctrl selects the highlighted tab.

use crate::workspace::TabId;

/// Most tabs a switcher shows (the most recent ones).
pub const MAX_SWITCHER_TABS: usize = 12;

/// Tabs, most recently used first. Not persisted; seeded from the tabs'
/// last-access times at startup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentTabs {
    order: Vec<TabId>,
}

impl RecentTabs {
    /// Order `(tab, last accessed)` pairs by recency, newest first.
    pub fn seeded(tabs: &[(TabId, f64)]) -> RecentTabs {
        let mut tabs = tabs.to_vec();
        tabs.sort_by(|a, b| b.1.total_cmp(&a.1));
        RecentTabs {
            order: tabs.into_iter().map(|(id, _)| id).collect(),
        }
    }

    /// `id` is being looked at now.
    pub fn touch(&mut self, id: TabId) {
        self.order.retain(|t| *t != id);
        self.order.insert(0, id);
    }

    /// `candidates` (e.g. the tabs of the active space) in recency order;
    /// tabs never looked at follow in their given order.
    pub fn order(&self, candidates: &[TabId]) -> Vec<TabId> {
        let mut out: Vec<TabId> = self.order.iter().copied().filter(|t| candidates.contains(t)).collect();
        out.extend(candidates.iter().copied().filter(|t| !self.order.contains(t)));
        out
    }

    /// Drop tabs that no longer exist.
    pub fn retain(&mut self, exists: impl Fn(TabId) -> bool) {
        self.order.retain(|t| exists(*t));
    }
}

/// One Ctrl+Tab session.
#[derive(Clone, Debug, PartialEq)]
pub struct Switcher {
    tabs: Vec<TabId>,
    index: usize,
}

impl Switcher {
    /// Start over `order` (most recent first, the current tab at its head),
    /// one step in `direction` (1 = Ctrl+Tab, -1 = Ctrl+Shift+Tab) from the
    /// current tab. `None` when there is nothing to switch to.
    pub fn start(mut order: Vec<TabId>, current: Option<TabId>, direction: isize) -> Option<Switcher> {
        if let Some(current) = current
            && let Some(i) = order.iter().position(|t| *t == current)
        {
            let tab = order.remove(i);
            order.insert(0, tab);
        }
        order.truncate(MAX_SWITCHER_TABS);
        if order.len() < 2 {
            return None;
        }
        let mut s = Switcher { tabs: order, index: 0 };
        s.step(direction);
        Some(s)
    }

    /// Move the highlight, wrapping around.
    pub fn step(&mut self, delta: isize) {
        let n = self.tabs.len() as isize;
        self.index = (self.index as isize + delta).rem_euclid(n) as usize;
    }

    /// Highlight a tab directly (a click or hover in the overlay).
    pub fn highlight(&mut self, id: TabId) {
        if let Some(i) = self.tabs.iter().position(|t| *t == id) {
            self.index = i;
        }
    }

    pub fn tabs(&self) -> &[TabId] {
        &self.tabs
    }

    pub fn index(&self) -> usize {
        self.index
    }

    /// The tab releasing Ctrl selects.
    pub fn selected(&self) -> TabId {
        self.tabs[self.index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ids(n: usize) -> Vec<TabId> {
        (0..n).map(|_| Uuid::new_v4()).collect()
    }

    #[test]
    fn recency_order_with_unknown_tabs_last() {
        let t = ids(4);
        let mut recent = RecentTabs::seeded(&[(t[0], 10.0), (t[1], 30.0), (t[2], 20.0)]);
        assert_eq!(recent.order(&t), vec![t[1], t[2], t[0], t[3]]);
        recent.touch(t[3]);
        recent.touch(t[0]);
        assert_eq!(recent.order(&t), vec![t[0], t[3], t[1], t[2]]);
        // Only the candidates (another space's tabs are left out).
        assert_eq!(recent.order(&[t[2], t[3]]), vec![t[3], t[2]]);
        recent.retain(|id| id != t[3]);
        assert_eq!(recent.order(&t[..3]), vec![t[0], t[1], t[2]]);
    }

    #[test]
    fn ctrl_tab_starts_on_the_previous_tab_and_wraps() {
        let t = ids(3);
        let mut s = Switcher::start(t.clone(), Some(t[0]), 1).unwrap();
        assert_eq!(s.selected(), t[1], "a quick Ctrl+Tab goes back to the last tab");
        s.step(1);
        assert_eq!(s.selected(), t[2]);
        s.step(1);
        assert_eq!(s.selected(), t[0], "wraps to the current tab");
        let back = Switcher::start(t.clone(), Some(t[0]), -1).unwrap();
        assert_eq!(back.selected(), t[2], "Ctrl+Shift+Tab starts at the oldest");
    }

    #[test]
    fn the_current_tab_leads_even_if_not_most_recent() {
        // A split pane's tab can be the current one without being first.
        let t = ids(3);
        let s = Switcher::start(t.clone(), Some(t[2]), 1).unwrap();
        assert_eq!(s.tabs(), &[t[2], t[0], t[1]]);
        assert_eq!(s.selected(), t[0]);
    }

    #[test]
    fn nothing_to_switch_to() {
        let t = ids(1);
        assert!(Switcher::start(t.clone(), Some(t[0]), 1).is_none());
        assert!(Switcher::start(Vec::new(), None, 1).is_none());
    }

    #[test]
    fn long_lists_are_capped_and_tabs_can_be_picked() {
        let t = ids(MAX_SWITCHER_TABS + 5);
        let mut s = Switcher::start(t.clone(), Some(t[0]), 1).unwrap();
        assert_eq!(s.tabs().len(), MAX_SWITCHER_TABS);
        s.highlight(t[3]);
        assert_eq!((s.index(), s.selected()), (3, t[3]));
        // A tab beyond the cap isn't in the list: the highlight stays.
        s.highlight(t[MAX_SWITCHER_TABS + 1]);
        assert_eq!(s.selected(), t[3]);
    }
}
