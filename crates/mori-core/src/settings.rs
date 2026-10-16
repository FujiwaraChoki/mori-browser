//! User preferences (`settings.json`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::paths;

/// File name inside the data directory.
pub const SETTINGS_FILE: &str = "settings.json";

/// Default sidebar width in points (Zen's: with the 8px gutter the web
/// card starts at x = 186).
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 178.0;
/// Narrowest the sidebar may be dragged.
pub const SIDEBAR_MIN_WIDTH: f32 = 160.0;
/// Widest the sidebar may be dragged.
pub const SIDEBAR_MAX_WIDTH: f32 = 420.0;

/// "Sleep idle tabs" choices: (minutes, label). 0 = never.
pub const SLEEP_OPTIONS: [(u32, &str); 6] = [
    (0, "Never"),
    (15, "15 minutes"),
    (30, "30 minutes"),
    (60, "1 hour"),
    (180, "3 hours"),
    (360, "6 hours"),
];

/// "Archive idle tabs" choices: (hours, label). 0 = never.
pub const ARCHIVE_OPTIONS: [(u32, &str); 6] = [
    (0, "Never"),
    (12, "12 hours"),
    (24, "1 day"),
    (72, "3 days"),
    (168, "1 week"),
    (720, "30 days"),
];

/// What a new tab loads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NewTabBehavior {
    #[default]
    Homepage,
    Blank,
}

/// The default search engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngine {
    #[default]
    Google,
    DuckDuckGo,
    Bing,
    Brave,
    Custom,
}

impl SearchEngine {
    /// Every engine, in picker order.
    pub const ALL: [SearchEngine; 5] = [
        SearchEngine::Google,
        SearchEngine::DuckDuckGo,
        SearchEngine::Bing,
        SearchEngine::Brave,
        SearchEngine::Custom,
    ];

    /// Human-readable name for the settings picker.
    pub fn label(self) -> &'static str {
        match self {
            SearchEngine::Google => "Google",
            SearchEngine::DuckDuckGo => "DuckDuckGo",
            SearchEngine::Bing => "Bing",
            SearchEngine::Brave => "Brave",
            SearchEngine::Custom => "Custom…",
        }
    }

    /// The query template (`{query}` placeholder); empty for [`SearchEngine::Custom`].
    pub fn template(self) -> &'static str {
        match self {
            SearchEngine::Google => "https://www.google.com/search?q={query}",
            SearchEngine::DuckDuckGo => "https://duckduckgo.com/?q={query}",
            SearchEngine::Bing => "https://www.bing.com/search?q={query}",
            SearchEngine::Brave => "https://search.brave.com/search?q={query}",
            SearchEngine::Custom => "",
        }
    }
}

/// Light/dark preference for the chrome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// Resolve against the OS preference.
    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Appearance::System => system_dark,
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }
}

/// Which window edge the sidebar sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
    #[default]
    Left,
    Right,
}

/// All user preferences. Missing keys fall back to their defaults.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub homepage_url: String,
    pub new_tab_behavior: NewTabBehavior,
    pub search_engine: SearchEngine,
    pub custom_search_template: String,
    pub ai_integration_enabled: bool,
    pub appearance: Appearance,
    pub show_sidebar_on_launch: bool,
    pub sidebar_position: SidebarPosition,
    pub sidebar_width: f32,
    pub auto_pip: bool,
    /// 0 = never sleep idle tabs.
    pub auto_sleep_minutes: u32,
    /// 0 = never archive idle tabs.
    pub auto_archive_hours: u32,
    pub agent_auto_approve_safe_reads: bool,
    /// Last sidebar visibility (restored when `show_sidebar_on_launch` is off).
    pub sidebar_visible: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            homepage_url: "mori://newtab".into(),
            new_tab_behavior: NewTabBehavior::Homepage,
            search_engine: SearchEngine::Google,
            custom_search_template: "https://www.example.com/search?q={query}".into(),
            ai_integration_enabled: true,
            appearance: Appearance::System,
            show_sidebar_on_launch: true,
            sidebar_position: SidebarPosition::Left,
            sidebar_width: SIDEBAR_DEFAULT_WIDTH,
            auto_pip: true,
            auto_sleep_minutes: 60,
            auto_archive_hours: 24,
            agent_auto_approve_safe_reads: false,
            sidebar_visible: true,
        }
    }
}

impl Settings {
    /// Load `settings.json` from `dir`, or defaults when missing/unreadable.
    pub fn load(dir: &Path) -> Settings {
        paths::read_json(&dir.join(SETTINGS_FILE)).unwrap_or_default()
    }

    /// Persist to `settings.json` in `dir`.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(SETTINGS_FILE), self)
    }

    /// Sidebar width clamped to the allowed range (NaN → default).
    pub fn clamped_sidebar_width(&self) -> f32 {
        if self.sidebar_width.is_nan() {
            return SIDEBAR_DEFAULT_WIDTH;
        }
        self.sidebar_width
            .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
    }

    /// The URL a new tab should load.
    pub fn new_tab_url(&self) -> &str {
        match self.new_tab_behavior {
            NewTabBehavior::Homepage => &self.homepage_url,
            NewTabBehavior::Blank => "about:blank",
        }
    }

    /// The search-results URL for `query` with the configured engine.
    ///
    /// A blank custom template falls back to Google; a template without
    /// `{query}` gets the encoded query appended.
    pub fn search_url(&self, query: &str) -> String {
        let encoded = encode_query(query);
        let raw = match self.search_engine {
            SearchEngine::Custom => self.custom_search_template.trim(),
            engine => engine.template(),
        };
        let template = if raw.is_empty() {
            SearchEngine::Google.template()
        } else {
            raw
        };
        if template.contains("{query}") {
            template.replace("{query}", &encoded)
        } else {
            format!("{template}{encoded}")
        }
    }
}

/// Percent-encode a search query: everything except ASCII alphanumerics and
/// `-._~` becomes `%XX` (UTF-8 bytes, uppercase hex; space → `%20`).
pub fn encode_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    for byte in query.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn defaults_roundtrip() {
        let tmp = TempDir::new();
        assert_eq!(Settings::load(tmp.path()), Settings::default());
        let s = Settings {
            search_engine: SearchEngine::DuckDuckGo,
            sidebar_width: 999.0,
            ..Settings::default()
        };
        s.save(tmp.path()).unwrap();
        let loaded = Settings::load(tmp.path());
        assert_eq!(loaded, s);
        assert_eq!(loaded.clamped_sidebar_width(), SIDEBAR_MAX_WIDTH);
    }

    #[test]
    fn partial_json_uses_defaults() {
        let s: Settings = serde_json::from_str(r#"{"search_engine":"brave"}"#).unwrap();
        assert_eq!(s.search_engine, SearchEngine::Brave);
        assert_eq!(s.auto_sleep_minutes, 60);
        assert_eq!(s.homepage_url, "mori://newtab");
    }

    #[test]
    fn search_urls() {
        let mut s = Settings::default();
        assert_eq!(
            s.search_url("rust lang&co"),
            "https://www.google.com/search?q=rust%20lang%26co"
        );
        s.search_engine = SearchEngine::DuckDuckGo;
        assert_eq!(
            s.search_url("a+b/c#d"),
            "https://duckduckgo.com/?q=a%2Bb%2Fc%23d"
        );
        s.search_engine = SearchEngine::Custom;
        s.custom_search_template = "https://x.test/s?term={query}&x=1".into();
        assert_eq!(s.search_url("ü"), "https://x.test/s?term=%C3%BC&x=1");
        s.custom_search_template = "https://x.test/s?q=".into();
        assert_eq!(s.search_url("hi"), "https://x.test/s?q=hi");
        s.custom_search_template = "  ".into();
        assert_eq!(s.search_url("hi"), "https://www.google.com/search?q=hi");
    }
}
