//! Page zoom: Chrome's zoom steps and the per-site zoom levels Mori
//! remembers (`zoom.json`).
//!
//! Like Chrome, zoom belongs to the site (host), not the tab: every tab on a
//! host shares it and it survives restarts. Chromium measures zoom in
//! "levels" where each level is a factor of 1.2; the UI speaks percent.

use std::{collections::BTreeMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::paths;

pub const ZOOM_FILE: &str = "zoom.json";

/// Chrome's zoom presets, as factors.
pub const PRESETS: &[f64] = &[
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

/// Two zoom values closer than this are the same step.
const EPSILON: f64 = 0.001;

/// Chromium zoom level for a zoom factor (`1.0` = 100% = level 0).
pub fn level_for_factor(factor: f64) -> f64 {
    factor.max(f64::MIN_POSITIVE).ln() / 1.2f64.ln()
}

/// Zoom factor for a Chromium zoom level.
pub fn factor_for_level(level: f64) -> f64 {
    1.2f64.powf(level)
}

/// Rounded percentage for display, e.g. `125`.
pub fn percent_for_level(level: f64) -> u32 {
    (factor_for_level(level) * 100.0).round() as u32
}

/// The next preset above (`direction > 0`) or below the current level.
/// Levels between presets snap to the nearest preset in that direction;
/// at either end the level stays put. `direction == 0` resets to 100%.
pub fn step(level: f64, direction: i32) -> f64 {
    if direction == 0 {
        return 0.0;
    }
    let factor = factor_for_level(level);
    let next = if direction > 0 {
        PRESETS.iter().copied().find(|p| *p > factor + EPSILON)
    } else {
        PRESETS.iter().rev().copied().find(|p| *p < factor - EPSILON)
    };
    match next {
        Some(p) => level_for_factor(p),
        None => level,
    }
}

pub fn is_default(level: f64) -> bool {
    level.abs() < EPSILON
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoomStore {
    /// Host -> Chromium zoom level. Hosts at 100% are not stored.
    pub hosts: BTreeMap<String, f64>,
}

impl ZoomStore {
    pub fn load(dir: &Path) -> ZoomStore {
        paths::read_json(&dir.join(ZOOM_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(ZOOM_FILE), self)
    }

    /// The zoom level for `host` (0 when none is stored).
    pub fn level(&self, host: &str) -> f64 {
        self.hosts.get(host).copied().filter(|l| l.is_finite()).unwrap_or(0.0)
    }

    /// Remember `level` for `host`; 100% forgets it.
    pub fn set(&mut self, host: &str, level: f64) {
        if is_default(level) || !level.is_finite() {
            self.hosts.remove(host);
        } else {
            self.hosts.insert(host.to_string(), level);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    fn percent_steps(mut level: f64, direction: i32, n: usize) -> Vec<u32> {
        (0..n)
            .map(|_| {
                level = step(level, direction);
                percent_for_level(level)
            })
            .collect()
    }

    #[test]
    fn steps_through_chrome_presets() {
        assert_eq!(percent_steps(0.0, 1, 5), vec![110, 125, 150, 175, 200]);
        assert_eq!(percent_steps(0.0, -1, 4), vec![90, 80, 75, 67]);
        assert_eq!(step(level_for_factor(1.5), 0), 0.0);
    }

    #[test]
    fn stops_at_the_ends() {
        let max = level_for_factor(5.0);
        assert!((step(max, 1) - max).abs() < 1e-9);
        let min = level_for_factor(0.25);
        assert!((step(min, -1) - min).abs() < 1e-9);
    }

    #[test]
    fn off_preset_levels_snap_in_the_step_direction() {
        // 131% (three old 0.5-level steps) goes up to 150% and down to 125%.
        let odd = 1.5;
        assert_eq!(percent_for_level(odd), 131);
        assert_eq!(percent_for_level(step(odd, 1)), 150);
        assert_eq!(percent_for_level(step(odd, -1)), 125);
    }

    #[test]
    fn level_factor_roundtrip() {
        for p in PRESETS {
            assert!((factor_for_level(level_for_factor(*p)) - p).abs() < 1e-9);
        }
        assert!(is_default(level_for_factor(1.0)));
    }

    #[test]
    fn store_forgets_default_and_persists() {
        let tmp = TempDir::new();
        let mut store = ZoomStore::default();
        store.set("example.com", level_for_factor(1.25));
        store.set("other.org", 0.0);
        assert_eq!(percent_for_level(store.level("example.com")), 125);
        assert_eq!(store.level("other.org"), 0.0);
        assert!(!store.hosts.contains_key("other.org"));
        store.save(tmp.path()).unwrap();
        assert_eq!(ZoomStore::load(tmp.path()), store);
        store.set("example.com", 0.0);
        assert!(store.hosts.is_empty());
    }
}
