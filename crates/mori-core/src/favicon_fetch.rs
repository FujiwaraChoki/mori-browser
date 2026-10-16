//! Finding a site's icon without loading the page in a tab.
//!
//! Saved tabs (favorites, pinned tabs, imported spaces) are usually not
//! loaded, so Chromium never reports their favicon. The chrome fetches the
//! site's home page instead and picks the icon it declares, falling back to
//! `/favicon.ico`. This module holds the rules: which hosts are worth a
//! fetch and which `<link>` to use.

use url::Url;

/// Whether `host` may be looked up on the network. Local and private
/// addresses are left alone, so a saved router or dev-server tab never
/// triggers requests on its own.
pub fn fetchable_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    if host.is_empty() || !host.contains('.') || host.ends_with(".local") || host.ends_with(".localhost") {
        return false;
    }
    match host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        Ok(_) => false,
        Err(_) => host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'),
    }
}

/// Icon URLs to try for a page, best first: icons declared in `html`
/// (bitmap `rel="icon"` before `apple-touch-icon`; SVG and mask icons are
/// skipped because they can't be decoded), then `/favicon.ico`.
pub fn icon_candidates(page_url: &str, html: &str) -> Vec<String> {
    let Ok(base) = Url::parse(page_url) else {
        return Vec::new();
    };
    let mut icons = Vec::new();
    let mut touch = Vec::new();
    for tag in link_tags(html) {
        let Some(rel) = attr(tag, "rel").map(|r| r.to_ascii_lowercase()) else {
            continue;
        };
        let Some(href) = attr(tag, "href") else {
            continue;
        };
        let rels: Vec<&str> = rel.split_ascii_whitespace().collect();
        let is_svg = attr(tag, "type").is_some_and(|t| t.contains("svg"))
            || href.split(['?', '#']).next().is_some_and(|p| p.to_ascii_lowercase().ends_with(".svg"));
        if is_svg || rels.contains(&"mask-icon") {
            continue;
        }
        let Ok(url) = base.join(href.trim()) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https") {
            continue;
        }
        if rels.contains(&"icon") {
            icons.push(url.to_string());
        } else if rels.iter().any(|r| r.starts_with("apple-touch-icon")) {
            touch.push(url.to_string());
        }
    }
    icons.extend(touch);
    if let Ok(ico) = base.join("/favicon.ico") {
        icons.push(ico.to_string());
    }
    icons.dedup();
    icons
}

/// The `<link ...>` tags of `html`, up to the end of its `<head>`.
fn link_tags(html: &str) -> impl Iterator<Item = &str> {
    let lower = html.to_ascii_lowercase();
    let end = lower.find("</head>").unwrap_or(html.len());
    let mut pos = 0;
    std::iter::from_fn(move || {
        let start = pos + lower[pos..end].find("<link")?;
        let close = start + lower[start..end].find('>').unwrap_or(end - start);
        pos = close.min(end);
        Some(&html[start..close])
    })
}

/// The value of attribute `name` in a tag, quoted or not.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let value_start = tag.len() - rest.len() + 1;
        let value = tag[value_start..].trim_start();
        return Some(match value.chars().next()? {
            q @ ('"' | '\'') => value[1..].split(q).next().unwrap_or(""),
            _ => value.split(|c: char| c.is_ascii_whitespace() || c == '>').next().unwrap_or(""),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_icons_come_first_then_touch_icons_then_favicon_ico() {
        let html = r#"<html><head>
            <link rel="apple-touch-icon" href="/apple.png">
            <link rel="stylesheet" href="/x.css">
            <link rel="icon" type="image/svg+xml" href="/icon.svg">
            <link rel="mask-icon" href="/mask.png">
            <LINK REL="shortcut icon" HREF="https://cdn.example.net/fav.png?v=2">
            <link href='/i32.png' rel='icon' sizes=32x32>
            </head><body><link rel="icon" href="/late.png"></body></html>"#;
        assert_eq!(
            icon_candidates("https://example.com/some/page", html),
            vec![
                "https://cdn.example.net/fav.png?v=2",
                "https://example.com/i32.png",
                "https://example.com/apple.png",
                "https://example.com/favicon.ico",
            ]
        );
    }

    #[test]
    fn falls_back_to_favicon_ico_and_ignores_non_http_icons() {
        let html = r#"<head><link rel="icon" href="data:image/png;base64,AAAA"></head>"#;
        assert_eq!(icon_candidates("https://a.example.org/", html), vec!["https://a.example.org/favicon.ico"]);
        assert!(icon_candidates("not a url", html).is_empty());
    }

    #[test]
    fn relative_icons_resolve_against_the_page() {
        let html = r#"<link rel=icon href=img/f.png>"#;
        assert_eq!(icon_candidates("https://example.com/docs/", html)[0], "https://example.com/docs/img/f.png");
    }

    #[test]
    fn only_public_names_are_fetched() {
        assert!(fetchable_host("github.com"));
        assert!(fetchable_host("mail.google.com"));
        assert!(!fetchable_host("localhost"));
        assert!(!fetchable_host("router"));
        assert!(!fetchable_host("192.168.1.1"));
        assert!(!fetchable_host("[::1]"));
        assert!(!fetchable_host("printer.local"));
        assert!(!fetchable_host("app.localhost"));
        assert!(!fetchable_host(""));
    }
}
