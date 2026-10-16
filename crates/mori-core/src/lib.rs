//! Mori's browser core: persisted state, product rules and page scripts,
//! with no UI or engine dependencies so everything here is unit-testable.
//!
//! Every store is a plain value with `load(dir)` / `save(dir)` against the
//! directory from [`paths::data_dir`]; callers decide when to save.
//! Timestamps are `f64` UNIX seconds ([`paths::now`]).

pub mod archive;
pub mod bookmarks;
pub mod boosts;
pub mod downloads;
pub mod history;
pub mod launcher;
pub mod maintenance;
pub mod navigation;
pub mod omnibox;
pub mod page_actions;
pub mod paths;
pub mod persist;
pub mod profiles;
pub mod prompts;
pub mod routes;
pub mod scripts;
pub mod security;
pub mod settings;
pub mod site_permissions;
pub mod theme;
pub mod util;
pub mod windows;
pub mod workspace;
pub mod zoom;

pub use archive::{ArchiveEntry, ArchiveGroup, ArchiveStore};
pub use bookmarks::{Bookmark, BookmarkStore};
pub use boosts::{Boost, BoostStore};
pub use history::{HistoryEntry, HistoryStore};
pub use launcher::{Candidate, ItemKind};
pub use omnibox::Resolved;
pub use paths::{data_dir, now};
pub use routes::{Route, RouteStore};
pub use scripts::MediaCommand;
pub use settings::{Appearance, NewTabBehavior, SearchEngine, Settings, SidebarPosition};
pub use theme::{GradientDot, GradientTheme, Palette, Preset, Rgba, SchemeOverride};
pub use util::host_of;
