//! Asset source: Mori's own files first, then the full Lucide icon set.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

pub struct MoriAssets;

const OWN: &[(&str, &[u8])] = &[
    ("mori/logo.svg", include_bytes!("../assets/mori/logo.svg")),
    // Two-tone folder glyphs for sidebar folders (Zen draws filled folders).
    ("mori/folder-filled.svg", include_bytes!("../assets/mori/folder-filled.svg")),
    ("mori/folder-open-filled.svg", include_bytes!("../assets/mori/folder-open-filled.svg")),
];

/// Lucide icons that were renamed upstream: the old names Mori uses map to
/// the files gpui-kit-assets ships today.
const RENAMED: &[(&str, &str)] = &[
    ("icons/trash-2.svg", "icons/trash.svg"),
    ("icons/text-select.svg", "icons/square-dashed-text.svg"),
];

impl AssetSource for MoriAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        let path = RENAMED
            .iter()
            .find(|(old, _)| *old == path)
            .map_or(path, |(_, new)| new);
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out = gpui_kit::assets::AllAssets.list(path)?;
        out.extend(
            OWN.iter()
                .map(|(p, _)| *p)
                .chain(RENAMED.iter().map(|(old, _)| *old))
                .filter(|p| p.starts_with(path))
                .map(SharedString::from),
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls whose argument at the given index (0-based) names a Lucide icon.
    const ICON_ARGS: &[(&str, usize)] = &[
        ("icon(", 0),
        ("icon_button(", 1),
        ("item(", 1),
        ("cmd(", 1),
        ("add(", 3),
    ];

    /// The arguments of the call starting right after `open` paren, split at
    /// top-level commas.
    fn call_args(src: &str) -> Vec<&str> {
        let (mut depth, mut in_str, mut start) = (0i32, false, 0usize);
        let mut args = Vec::new();
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if in_str {
                if c == b'\\' {
                    i += 1;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' if depth == 0 => {
                        args.push(&src[start..i]);
                        return args;
                    }
                    b')' | b']' | b'}' => depth -= 1,
                    b',' if depth == 0 => {
                        args.push(&src[start..i]);
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            i += 1;
        }
        args
    }

    /// String literals shaped like icon names (`arrow-left`, `x`).
    fn icon_literals(arg: &str) -> Vec<&str> {
        arg.split('"')
            .skip(1)
            .step_by(2)
            .filter(|s| {
                !s.is_empty()
                    && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            })
            .collect()
    }

    fn sources(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((path.display().to_string(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }

    #[test]
    fn every_icon_used_in_the_ui_exists() {
        let mut files = Vec::new();
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        let mut names = Vec::new();
        for (file, src) in &files {
            for (call, index) in ICON_ARGS {
                for (at, _) in src.match_indices(call) {
                    // Skip longer identifiers ending in the call name
                    // (`add_icon(`), and the definitions themselves.
                    let before = src[..at].chars().last();
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_')
                        || src[..at].ends_with("fn ")
                    {
                        continue;
                    }
                    let args = call_args(&src[at + call.len()..]);
                    if let Some(arg) = args.get(*index) {
                        for name in icon_literals(arg) {
                            names.push((file.clone(), name.to_string()));
                        }
                    }
                }
            }
        }
        for symbol in crate::ui::sidebar::SPACE_SYMBOLS {
            names.push(("SPACE_SYMBOLS".into(), symbol.to_string()));
        }
        for symbol in mori_core::workspace::NEW_SPACE_SYMBOLS {
            // New spaces must get a symbol the space editor offers too.
            assert!(crate::ui::sidebar::SPACE_SYMBOLS.contains(&symbol), "{symbol} not in SPACE_SYMBOLS");
            names.push(("NEW_SPACE_SYMBOLS".into(), symbol.to_string()));
        }
        assert!(names.len() > 50, "scanner found only {} icon names", names.len());
        let missing: Vec<_> = names
            .iter()
            .filter(|(_, n)| {
                MoriAssets
                    .load(&format!("icons/{n}.svg"))
                    .ok()
                    .flatten()
                    .is_none()
            })
            .collect();
        assert!(missing.is_empty(), "icons without an asset: {missing:?}");
    }
}
