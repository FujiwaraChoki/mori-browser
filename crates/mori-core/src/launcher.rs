//! Launcher (⌘T / ⌘L palette) matching and ranking, independent of the UI.
//!
//! The UI builds [`Candidate`]s from open tabs, history, bookmarks and
//! commands, then calls [`rank`]. The fixed "Open/Search X" first row and the
//! trailing "Ask AI" row are the UI's job (see [`crate::omnibox`]); budget
//! `limit` accordingly (the launcher shows [`MAX_ROWS`] rows in total).

use std::collections::HashMap;

/// Total rows the launcher shows, including the fixed first/last rows.
pub const MAX_ROWS: usize = 8;
/// History entries offered when the query is empty.
pub const IDLE_HISTORY: usize = 8;

/// Where a launcher row comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Tab,
    History,
    Bookmark,
    Command,
}

impl ItemKind {
    /// Ranking bonus added to the match score.
    pub fn bonus(self) -> f32 {
        match self {
            ItemKind::Tab => 70.0,
            ItemKind::History => 55.0,
            ItemKind::Bookmark => 45.0,
            ItemKind::Command => 0.0,
        }
    }
}

/// One rankable launcher row.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// Unique id, e.g. a tab id, URL or command id.
    pub key: String,
    pub kind: ItemKind,
    pub title: String,
    /// Used for matching (secondary text) and de-duplication.
    pub url: Option<String>,
    /// Extra secondary match text (commands' keywords).
    pub keywords: String,
    /// UNIX seconds; newer wins ties.
    pub recency: f64,
}

/// A candidate with its final rank (`score + bonus`).
#[derive(Clone, Debug, PartialEq)]
pub struct Ranked {
    pub candidate: Candidate,
    pub rank: f32,
}

/// Match `query` against a title and secondary text (URL / keywords).
///
/// The query is split on spaces; every token must match either the title or
/// the secondary text. Per token, the best of the title score and the
/// secondary score − 25 counts: prefix 400, word-boundary 340, substring 260,
/// in-order subsequence (tokens of ≥ 3 chars) 180. The result is the
/// (integer) average across tokens; `None` if any token misses.
pub fn score(query: &str, title: &str, secondary: &str) -> Option<f32> {
    let query = query.to_lowercase();
    let tokens: Vec<&str> = query.split(' ').filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return None;
    }
    let title = title.to_lowercase();
    let secondary = secondary.to_lowercase();
    let mut total: u32 = 0;
    for token in &tokens {
        let t = token_score(token, &title);
        let s = token_score(token, &secondary).map(|v| v.saturating_sub(25));
        total += t.max(s)?;
    }
    Some((total / tokens.len() as u32) as f32)
}

fn token_score(needle: &str, haystack: &str) -> Option<u32> {
    if needle.is_empty() || haystack.is_empty() {
        return None;
    }
    if haystack.starts_with(needle) {
        Some(400)
    } else if has_word_boundary_match(needle, haystack) {
        Some(340)
    } else if haystack.contains(needle) {
        Some(260)
    } else if needle.chars().count() >= 3 && is_subsequence(needle, haystack) {
        Some(180)
    } else {
        None
    }
}

fn has_word_boundary_match(needle: &str, haystack: &str) -> bool {
    haystack.match_indices(needle).any(|(idx, _)| {
        haystack[..idx]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric())
    })
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars();
    needle.chars().all(|c| hay.any(|h| h == c))
}

fn secondary_text(c: &Candidate) -> String {
    match (
        c.url.as_deref().filter(|u| !u.is_empty()),
        c.keywords.is_empty(),
    ) {
        (Some(url), true) => url.to_string(),
        (Some(url), false) => format!("{url} {}", c.keywords),
        (None, _) => c.keywords.clone(),
    }
}

/// Rank candidates for `query`, best first, at most `limit`.
///
/// - Empty query: only tabs and the [`IDLE_HISTORY`] most recent history
///   entries, ordered by kind bonus (tabs first) then recency.
/// - Otherwise every kind is matched with [`score`] plus its
///   [`ItemKind::bonus`]. Commands with a query under 3 characters need ≥ 2
///   characters and a title prefix match, and score at most 240.
/// - Duplicate URLs (and keys) collapse to the best-ranked one.
/// - Ties: newer `recency` first, then title (case-insensitive) ascending.
pub fn rank(query: &str, candidates: Vec<Candidate>, limit: usize) -> Vec<Candidate> {
    rank_with_scores(query, candidates, limit)
        .into_iter()
        .map(|r| r.candidate)
        .collect()
}

/// [`rank`], keeping each row's final rank value.
pub fn rank_with_scores(query: &str, candidates: Vec<Candidate>, limit: usize) -> Vec<Ranked> {
    let q = query.trim().to_lowercase();
    let mut ranked: Vec<Ranked> = if q.is_empty() {
        idle_candidates(candidates)
            .into_iter()
            .map(|c| Ranked {
                rank: c.kind.bonus(),
                candidate: c,
            })
            .collect()
    } else {
        candidates
            .into_iter()
            .filter_map(|c| {
                let s = match_candidate(&q, &c)?;
                Some(Ranked {
                    rank: s + c.kind.bonus(),
                    candidate: c,
                })
            })
            .collect()
    };

    sort_ranked(&mut ranked);
    dedupe(&mut ranked);
    ranked.truncate(limit);
    ranked
}

fn idle_candidates(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let (mut history, rest): (Vec<_>, Vec<_>) = candidates
        .into_iter()
        .filter(|c| matches!(c.kind, ItemKind::Tab | ItemKind::History))
        .partition(|c| c.kind == ItemKind::History);
    history.sort_by(|a, b| b.recency.total_cmp(&a.recency));
    history.truncate(IDLE_HISTORY);
    rest.into_iter().chain(history).collect()
}

fn match_candidate(q: &str, c: &Candidate) -> Option<f32> {
    let s = score(q, &c.title, &secondary_text(c))?;
    if c.kind == ItemKind::Command && q.chars().count() < 3 {
        if q.chars().count() < 2 || score(q, &c.title, "")? < 400.0 {
            return None;
        }
        return Some(s.min(240.0));
    }
    Some(s)
}

fn sort_ranked(ranked: &mut [Ranked]) {
    ranked.sort_by(|a, b| {
        b.rank
            .total_cmp(&a.rank)
            .then(b.candidate.recency.total_cmp(&a.candidate.recency))
            .then_with(|| {
                a.candidate
                    .title
                    .to_lowercase()
                    .cmp(&b.candidate.title.to_lowercase())
            })
    });
}

/// Drop later (worse-ranked) rows sharing a URL or key with an earlier one.
fn dedupe(ranked: &mut Vec<Ranked>) {
    let mut seen_urls: HashMap<String, ()> = HashMap::new();
    let mut seen_keys: HashMap<String, ()> = HashMap::new();
    ranked.retain(|r| {
        if seen_keys.insert(r.candidate.key.clone(), ()).is_some() {
            return false;
        }
        match r.candidate.url.as_deref().filter(|u| !u.is_empty()) {
            Some(url) => seen_urls.insert(url.to_string(), ()).is_none(),
            None => true,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(kind: ItemKind, title: &str, url: Option<&str>, recency: f64) -> Candidate {
        Candidate {
            key: format!("{kind:?}:{title}:{}", url.unwrap_or("")),
            kind,
            title: title.into(),
            url: url.map(Into::into),
            keywords: String::new(),
            recency,
        }
    }

    fn cmd(title: &str, keywords: &str) -> Candidate {
        Candidate {
            keywords: keywords.into(),
            ..cand(ItemKind::Command, title, None, 0.0)
        }
    }

    #[test]
    fn scoring_tiers() {
        assert_eq!(score("git", "GitHub", ""), Some(400.0));
        assert_eq!(score("hub", "Git-Hub Home", ""), Some(340.0));
        assert_eq!(score("hub", "GitHub", ""), Some(260.0));
        assert_eq!(score("ghb", "GitHub", ""), Some(180.0));
        assert_eq!(
            score("gh", "GitHub", ""),
            None,
            "subsequence needs ≥ 3 chars"
        );
        assert_eq!(score("git", "Code", "https://github.com"), Some(315.0));
        assert_eq!(score("hub", "Code", "https://github.com"), Some(235.0));
        assert_eq!(score("github", "Code", "github.com"), Some(375.0));
        // Title beats secondary.
        assert_eq!(score("code", "Code", "code.com"), Some(400.0));
        // All tokens must match; average (integer).
        assert_eq!(score("git zzz", "GitHub", ""), None);
        assert_eq!(score("git hub", "GitHub", ""), Some(330.0));
        assert_eq!(score("  ", "x", "y"), None);
        assert_eq!(score("RUST", "rust", ""), Some(400.0));
    }

    #[test]
    fn bonuses_and_order() {
        let cands = vec![
            cand(
                ItemKind::History,
                "Rust Book",
                Some("https://doc.rust-lang.org/book"),
                5.0,
            ),
            cand(
                ItemKind::Tab,
                "Rust Playground",
                Some("https://play.rust-lang.org"),
                1.0,
            ),
            cand(
                ItemKind::Bookmark,
                "Rust Blog",
                Some("https://blog.rust-lang.org"),
                9.0,
            ),
            cmd("Reader View", "reader read article"),
        ];
        let out = rank("rust", cands, 10);
        let titles: Vec<_> = out.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Rust Playground", "Rust Book", "Rust Blog"]);
    }

    #[test]
    fn dedupe_keeps_best() {
        let cands = vec![
            cand(
                ItemKind::History,
                "Example",
                Some("https://example.com/"),
                3.0,
            ),
            cand(ItemKind::Tab, "Example", Some("https://example.com/"), 1.0),
            cand(
                ItemKind::Bookmark,
                "Example",
                Some("https://example.com/"),
                2.0,
            ),
        ];
        let out = rank("exa", cands, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, ItemKind::Tab);
    }

    #[test]
    fn short_query_commands() {
        let cands = vec![
            cmd("Settings", "preferences options"),
            cmd("New Tab", "new tab open"),
        ];
        assert!(
            rank("s", cands.clone(), 10).is_empty(),
            "1 char never matches commands"
        );
        let out = rank_with_scores("se", cands.clone(), 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].candidate.title, "Settings");
        assert_eq!(out[0].rank, 240.0);
        // "op" matches keywords but not a title prefix → excluded.
        assert!(rank("op", cands.clone(), 10).is_empty());
        // ≥ 3 chars: keywords count, no cap.
        let out = rank_with_scores("pref", cands, 10);
        assert_eq!(out[0].rank, 375.0);
    }

    #[test]
    fn ties_by_recency_then_title() {
        let cands = vec![
            cand(ItemKind::History, "b docs", Some("https://b.com"), 1.0),
            cand(ItemKind::History, "a docs", Some("https://a.com"), 1.0),
            cand(ItemKind::History, "c docs", Some("https://c.com"), 2.0),
        ];
        let titles: Vec<_> = rank("docs", cands, 10)
            .into_iter()
            .map(|c| c.title)
            .collect();
        assert_eq!(titles, ["c docs", "a docs", "b docs"]);
    }

    #[test]
    fn empty_query_tabs_then_recent_history() {
        let mut cands = vec![
            cand(ItemKind::Tab, "Old tab", Some("https://t1.com"), 1.0),
            cand(ItemKind::Tab, "New tab", Some("https://t2.com"), 50.0),
            cand(ItemKind::Bookmark, "Mark", Some("https://m.com"), 99.0),
            cmd("Settings", ""),
        ];
        for i in 0..12 {
            cands.push(cand(
                ItemKind::History,
                &format!("h{i}"),
                Some(&format!("https://h{i}.com")),
                i as f64,
            ));
        }
        let out = rank("", cands, 100);
        assert_eq!(out.len(), 2 + IDLE_HISTORY);
        assert_eq!(out[0].title, "New tab");
        assert_eq!(out[1].title, "Old tab");
        assert_eq!(out[2].title, "h11");
        assert!(
            out.iter()
                .all(|c| matches!(c.kind, ItemKind::Tab | ItemKind::History))
        );
        assert_eq!(rank("", vec![], 5).len(), 0);
        let limited = rank(
            "",
            vec![
                cand(ItemKind::Tab, "a", None, 0.0),
                cand(ItemKind::Tab, "b", None, 0.0),
            ],
            1,
        );
        assert_eq!(limited.len(), 1);
    }
}
