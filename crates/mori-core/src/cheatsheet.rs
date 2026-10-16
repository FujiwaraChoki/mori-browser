//! The keyboard shortcut cheatsheet: searching and grouping the shortcut
//! list (which itself comes from the app's key binding registry).

/// One shortcut row: what it does, its section, and its keys as shown
/// ("Ctrl+Shift+T"; several bindings joined with " / ").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub title: String,
    pub group: &'static str,
    pub keys: Vec<String>,
}

/// Whether `entry` matches `query`: every word of the query appears in the
/// title, the section name or one of the key labels (case-insensitive).
/// "ctrl+t" or "Ctrl T" find shortcuts by their keys.
pub fn matches(entry: &Entry, query: &str) -> bool {
    let haystack = format!("{} {} {}", entry.title, entry.group, entry.keys.join(" ")).to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// `entries` matching `query`, grouped by section in first-seen order.
pub fn grouped<'a>(entries: &'a [Entry], query: &str) -> Vec<(&'static str, Vec<&'a Entry>)> {
    let mut out: Vec<(&'static str, Vec<&'a Entry>)> = Vec::new();
    for e in entries.iter().filter(|e| matches(e, query)) {
        match out.iter_mut().find(|(g, _)| *g == e.group) {
            Some((_, list)) => list.push(e),
            None => out.push((e.group, vec![e])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, group: &'static str, keys: &[&str]) -> Entry {
        Entry {
            title: title.into(),
            group,
            keys: keys.iter().map(|k| k.to_string()).collect(),
        }
    }

    #[test]
    fn search_by_words_and_keys() {
        let e = entry("Reopen Closed Tab", "Tabs", &["Ctrl+Shift+T"]);
        assert!(matches(&e, ""));
        assert!(matches(&e, "reopen"));
        assert!(matches(&e, "closed tab"));
        assert!(matches(&e, "tabs reopen"));
        assert!(matches(&e, "ctrl+shift+t"));
        assert!(matches(&e, "shift t"));
        assert!(!matches(&e, "reopen window"));
    }

    #[test]
    fn groups_keep_first_seen_order() {
        let list = vec![
            entry("New Tab", "Tabs", &["Ctrl+T"]),
            entry("Find", "Page", &["Ctrl+F"]),
            entry("Close Tab", "Tabs", &["Ctrl+W"]),
        ];
        let g = grouped(&list, "");
        assert_eq!(g.iter().map(|(name, l)| (*name, l.len())).collect::<Vec<_>>(), [("Tabs", 2), ("Page", 1)]);
        let g = grouped(&list, "close");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].1[0].title, "Close Tab");
        assert!(grouped(&list, "nothing like this").is_empty());
    }
}
