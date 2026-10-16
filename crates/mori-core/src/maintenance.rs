//! Idle-tab maintenance: which tabs auto-sleep unloads and which tabs
//! auto-archive closes.
//!
//! The UI stamps `last_accessed` while a tab is in use (selected, in the
//! visible split pane, or playing media) and marks tabs that must not lose
//! their page as busy; these functions only apply the idle cut-offs.

use crate::workspace::TabId;

/// What maintenance needs to know about one tab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabActivity {
    pub id: TabId,
    /// UNIX seconds of the last time the tab was in use.
    pub last_accessed: f64,
    /// Its page is loaded in Chromium.
    pub live: bool,
    /// Neither pinned nor in a folder.
    pub loose: bool,
    /// An http(s) page (internal pages are never archived).
    pub web: bool,
    /// On screen, playing media, waiting on a dialog or permission prompt,
    /// or downloading: unloading it would lose something the user sees.
    pub busy: bool,
}

/// Live, idle, non-busy tabs that should be put to sleep. `minutes == 0`
/// turns auto-sleep off.
pub fn sleep_candidates(tabs: &[TabActivity], now: f64, minutes: u32) -> Vec<TabId> {
    if minutes == 0 {
        return Vec::new();
    }
    let cutoff = now - minutes as f64 * 60.0;
    tabs.iter()
        .filter(|t| t.live && !t.busy && t.last_accessed < cutoff)
        .map(|t| t.id)
        .collect()
}

/// Loose, idle, non-busy web tabs that should be archived (closed and kept
/// in the archive). `hours == 0` turns auto-archive off.
pub fn archive_candidates(tabs: &[TabActivity], now: f64, hours: u32) -> Vec<TabId> {
    if hours == 0 {
        return Vec::new();
    }
    let cutoff = now - hours as f64 * 3600.0;
    tabs.iter()
        .filter(|t| t.loose && t.web && !t.busy && t.last_accessed < cutoff)
        .map(|t| t.id)
        .collect()
}

/// Page check run before a live tab is slept or archived. Evaluates to
/// `true` when the page holds something unloading would lose: text typed
/// into a form field or an edited `contenteditable` that still has focus.
pub const UNSAVED_INPUT_SCRIPT: &str = r#"(() => {
  try {
    const skip = new Set(['hidden', 'submit', 'button', 'reset', 'image', 'file']);
    for (const el of document.querySelectorAll('input, textarea')) {
      if (el.disabled || el.readOnly) continue;
      if (el.tagName === 'INPUT') {
        const type = (el.type || 'text').toLowerCase();
        if (skip.has(type)) continue;
        if (type === 'checkbox' || type === 'radio') {
          if (el.checked !== el.defaultChecked) return true;
          continue;
        }
      }
      if (el.value !== el.defaultValue && el.value.trim() !== '') return true;
    }
    for (const el of document.querySelectorAll('select')) {
      for (const o of el.options) if (o.selected !== o.defaultSelected) return true;
    }
    const active = document.activeElement;
    if (active && active.isContentEditable && (active.innerText || '').trim() !== '') return true;
    return false;
  } catch (e) {
    return false;
  }
})()"#;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const NOW: f64 = 1_000_000.0;

    fn tab(minutes_idle: f64) -> TabActivity {
        TabActivity {
            id: Uuid::new_v4(),
            last_accessed: NOW - minutes_idle * 60.0,
            live: true,
            loose: true,
            web: true,
            busy: false,
        }
    }

    #[test]
    fn sleeps_only_idle_live_quiet_tabs() {
        let idle = tab(90.0);
        let fresh = tab(10.0);
        let busy = TabActivity { busy: true, ..tab(500.0) };
        let asleep = TabActivity { live: false, ..tab(500.0) };
        let pinned = TabActivity { loose: false, ..tab(90.0) };
        let tabs = [idle, fresh, busy, asleep, pinned];
        assert_eq!(sleep_candidates(&tabs, NOW, 60), vec![idle.id, pinned.id]);
        assert!(sleep_candidates(&tabs, NOW, 0).is_empty(), "0 = never");
    }

    #[test]
    fn archives_only_idle_loose_web_quiet_tabs() {
        let day = 24.0 * 60.0;
        let stale = TabActivity { live: false, ..tab(2.0 * day) };
        let busy = TabActivity { busy: true, ..tab(2.0 * day) };
        let pinned = TabActivity { loose: false, ..tab(2.0 * day) };
        let internal = TabActivity { web: false, ..tab(2.0 * day) };
        let recent = tab(60.0);
        let tabs = [stale, busy, pinned, internal, recent];
        assert_eq!(archive_candidates(&tabs, NOW, 24), vec![stale.id]);
        assert!(archive_candidates(&tabs, NOW, 0).is_empty(), "0 = never");
    }
}
