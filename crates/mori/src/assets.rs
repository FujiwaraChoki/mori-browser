//! Asset source: Mori's own files first, then the full Lucide icon set.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

pub struct MoriAssets;

const OWN: &[(&str, &[u8])] = &[("mori/logo.svg", include_bytes!("../assets/mori/logo.svg"))];

impl AssetSource for MoriAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = OWN.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out = gpui_kit::assets::AllAssets.list(path)?;
        out.extend(
            OWN.iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        Ok(out)
    }
}
