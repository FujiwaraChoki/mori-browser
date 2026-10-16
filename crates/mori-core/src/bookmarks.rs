//! Bookmarks (`bookmarks.json`), newest first, toggled by URL.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::paths;

pub const BOOKMARKS_FILE: &str = "bookmarks.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: Uuid,
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// UNIX seconds.
    pub created_at: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BookmarkStore {
    pub bookmarks: Vec<Bookmark>,
}

impl BookmarkStore {
    pub fn load(dir: &Path) -> BookmarkStore {
        paths::read_json(&dir.join(BOOKMARKS_FILE)).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        paths::write_json_atomic(&dir.join(BOOKMARKS_FILE), self)
    }

    /// Add or remove `url`. Returns whether it is bookmarked afterwards.
    /// Blank pages can't be bookmarked. An empty title falls back to the URL.
    pub fn toggle(&mut self, url: &str, title: &str) -> bool {
        if url.is_empty() || url == "about:blank" {
            return false;
        }
        if let Some(idx) = self.bookmarks.iter().position(|b| b.url == url) {
            self.bookmarks.remove(idx);
            return false;
        }
        self.bookmarks.insert(
            0,
            Bookmark {
                id: Uuid::new_v4(),
                url: url.to_string(),
                title: if title.is_empty() {
                    url.to_string()
                } else {
                    title.to_string()
                },
                created_at: paths::now(),
            },
        );
        true
    }

    pub fn contains(&self, url: &str) -> bool {
        self.bookmarks.iter().any(|b| b.url == url)
    }

    /// All bookmarks, newest first.
    pub fn list(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Bookmark> {
        let idx = self.bookmarks.iter().position(|b| b.id == id)?;
        Some(self.bookmarks.remove(idx))
    }

    /// Case-insensitive substring filter on title or URL; empty → all.
    pub fn search(&self, query: &str) -> Vec<&Bookmark> {
        let q = query.trim().to_lowercase();
        self.bookmarks
            .iter()
            .filter(|b| {
                q.is_empty()
                    || b.title.to_lowercase().contains(&q)
                    || b.url.to_lowercase().contains(&q)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::test_support::TempDir;

    #[test]
    fn toggle_list_search() {
        let tmp = TempDir::new();
        let mut b = BookmarkStore::default();
        assert!(!b.toggle("about:blank", "x"));
        assert!(b.toggle("https://a.com/", "Alpha"));
        assert!(b.toggle("https://b.com/", ""));
        assert_eq!(b.list()[0].url, "https://b.com/");
        assert_eq!(b.list()[0].title, "https://b.com/");
        assert!(b.contains("https://a.com/"));
        assert_eq!(b.search("ALP").len(), 1);
        assert_eq!(b.search("").len(), 2);
        b.save(tmp.path()).unwrap();
        assert_eq!(BookmarkStore::load(tmp.path()), b);
        assert!(!b.toggle("https://a.com/", "Alpha"));
        assert!(!b.contains("https://a.com/"));
        let id = b.list()[0].id;
        assert!(b.remove(id).is_some());
        assert!(b.list().is_empty());
    }
}
