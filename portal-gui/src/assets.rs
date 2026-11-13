//! The icons and logos, compiled into the binary (assets/): nothing is read from disk at run time.
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! assets {
    ($($name:literal),* $(,)?) => {
        const FILES: &[(&str, &[u8])] = &[$(($name, include_bytes!(concat!("../assets/", $name)))),*];
    };
}

assets!(
    "logo.svg",
    "icons/app-window.svg", "icons/code.svg", "icons/terminal.svg", "icons/browser.svg", "icons/file-text.svg", "icons/chart-bar.svg", "icons/shield-check.svg",
    "icons/database.svg", "icons/image.svg", "icons/music-notes.svg", "icons/chat-circle.svg", "icons/wrench.svg", "icons/cube.svg", "icons/refresh.svg",
    "icons/search.svg", "icons/download.svg", "icons/trash.svg", "icons/check.svg", "icons/x.svg", "icons/warn.svg", "icons/info.svg", "icons/close.svg",
    "icons/list.svg", "icons/broom.svg", "icons/sun.svg", "icons/moon.svg", "icons/auto.svg", "icons/translate.svg",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES.iter().find(|(n, _)| *n == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES.iter().filter(|(n, _)| n.starts_with(path)).map(|(n, _)| SharedString::from(*n)).collect())
    }
}

/// The icon file for a catalog glyph name; an unknown name shows the cube.
pub fn icon_path(name: &str) -> String {
    let known = FILES.iter().any(|(n, _)| n.strip_prefix("icons/").and_then(|n| n.strip_suffix(".svg")) == Some(name));
    format!("icons/{}.svg", if known { name } else { "cube" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_glyphs_resolve_and_unknown_ones_fall_back_to_the_cube() {
        assert_eq!(icon_path("terminal"), "icons/terminal.svg");
        assert_eq!(icon_path("nope"), "icons/cube.svg");
        assert_eq!(icon_path(""), "icons/cube.svg");
        assert!(Assets.load("icons/check.svg").unwrap().is_some());
        assert!(Assets.load("icons/zzz.svg").unwrap().is_none());
    }
}
