//! Application icons of the app center. An icon is one of:
//! - a generic name from [`GENERIC`] (drawn by the interface),
//! - `brand:<slug>`, one of the bundled logos (Simple Icons, CC0): turned into an image here,
//! - an uploaded image as a `data:` URI (PNG, JPEG, WebP or SVG, small).
//!
//! Images are only ever shown through an `<img>` element, where an SVG cannot run scripts; uploads are still checked
//! (type, size, magic bytes, no script in an SVG) so that nothing odd is stored or opened on its own.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const GENERIC: &[&str] = &["", "app-window", "code", "terminal", "browser", "file-text", "chart-bar", "shield-check", "database", "image", "music-notes", "chat-circle", "wrench", "cube"];
pub const BRAND_PREFIX: &str = "brand:";
/// Largest uploaded image, as the base64 text of the data URI (about 72 KB of image).
pub const MAX_UPLOAD_B64: usize = 100_000;

#[derive(Deserialize)]
struct Brand {
    title: String,
    hex: String,
    path: String,
}

fn brands() -> &'static BTreeMap<String, Brand> {
    static B: OnceLock<BTreeMap<String, Brand>> = OnceLock::new();
    B.get_or_init(|| serde_json::from_str(include_str!("../assets/brand-icons.json")).expect("assets/brand-icons.json"))
}

fn svg_of(b: &Brand) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><path fill=\"#{}\" d=\"{}\"/></svg>", b.hex, b.path)
}

fn svg_uri(svg: &str) -> String {
    format!("data:image/svg+xml;base64,{}", STANDARD.encode(svg.as_bytes()))
}

/// The image of a `brand:<slug>` icon, as a data URI (None for the other kinds).
pub fn url(icon: &str) -> Option<String> {
    brands().get(icon.strip_prefix(BRAND_PREFIX)?).map(|b| svg_uri(&svg_of(b)))
}

/// The bundled logo of an application, by its name: `Firefox`, `git` or `Python 3` find theirs. For an app created
/// with no icon of its own (the generic default), so that a hand-written catalog entry looks like a suggested one.
pub fn guess(name: &str) -> Option<String> {
    let n = name.trim().to_lowercase();
    let bare = n.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ').trim_end();
    brands().iter().find(|(slug, b)| [n.as_str(), bare].iter().any(|k| !k.is_empty() && (*k == slug.as_str() || *k == b.title.to_lowercase()))).map(|(slug, _)| format!("{BRAND_PREFIX}{slug}"))
}

/// The bundled logos, for the icon picker of the catalog editor.
pub fn catalog() -> Vec<Value> {
    brands().iter().map(|(slug, b)| json!({ "slug": slug, "title": b.title, "hex": b.hex, "url": svg_uri(&svg_of(b)) })).collect()
}

/// Is this a value the catalog may store as an app's icon?
pub fn check(icon: &str) -> Result<(), String> {
    if GENERIC.contains(&icon) {
        return Ok(());
    }
    if let Some(slug) = icon.strip_prefix(BRAND_PREFIX) {
        return if brands().contains_key(slug) { Ok(()) } else { Err(format!("unknown brand icon '{slug}'")) };
    }
    let Some(rest) = icon.strip_prefix("data:") else {
        return Err(format!("icon must be one of: {}, a brand:<name> logo, or an uploaded image", GENERIC[1..].join(", ")));
    };
    let (mime, b64) = rest.split_once(";base64,").ok_or("uploaded icon must be a base64 data URI")?;
    if b64.len() > MAX_UPLOAD_B64 {
        return Err("uploaded icon is too large (72 KB at most)".into());
    }
    let bytes = STANDARD.decode(b64).map_err(|_| "uploaded icon is not valid base64".to_string())?;
    let ok = match mime {
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/webp" => bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "image/svg+xml" => svg_ok(&bytes),
        _ => return Err("uploaded icon must be a PNG, JPEG, WebP or SVG image".into()),
    };
    if ok { Ok(()) } else { Err(format!("the uploaded file is not a valid {mime} image")) }
}

fn svg_ok(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else { return false };
    let l = text.to_ascii_lowercase();
    l.contains("<svg")
        && !["<script", "javascript:", "<foreignobject", "<iframe", "<embed", "<object", "onload", "onerror", "onclick", "onmouse", "<!entity", "<!doctype"].iter().any(|bad| l.contains(bad))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uri(mime: &str, bytes: &[u8]) -> String {
        format!("data:{mime};base64,{}", STANDARD.encode(bytes))
    }

    #[test]
    fn generic_and_brand_icons_are_accepted() {
        assert!(check("").is_ok() && check("terminal").is_ok() && check("brand:git").is_ok());
        assert!(check("brand:no-such-logo").is_err() && check("brand:").is_err());
        assert!(check("https://example.org/x.png").is_err(), "no remote icons: nothing is fetched from elsewhere");
        assert!(check("javascript:alert(1)").is_err());
    }

    #[test]
    fn an_app_is_matched_to_its_logo_by_name() {
        assert_eq!(guess("Firefox").as_deref(), Some("brand:firefox"));
        assert_eq!(guess("  git ").as_deref(), Some("brand:git"));
        assert_eq!(guess("Python 3").as_deref(), Some("brand:python"), "a version number is ignored");
        assert_eq!(guess("Node.js").as_deref(), Some("brand:nodedotjs"), "or the logo's own title");
        assert_eq!(guess("OBS Studio").as_deref(), Some("brand:obsstudio"));
        assert!(guess("Outil interne").is_none() && guess("").is_none() && guess("3").is_none() && guess("ncdu").is_none());
    }

    #[test]
    fn a_brand_icon_becomes_a_coloured_svg_image() {
        let u = url("brand:git").unwrap();
        let svg = String::from_utf8(STANDARD.decode(u.strip_prefix("data:image/svg+xml;base64,").unwrap()).unwrap()).unwrap();
        assert!(svg.starts_with("<svg ") && svg.contains("fill=\"#F03C2E\"") && svg.contains("<path"));
        assert!(url("terminal").is_none() && url("brand:nope").is_none() && url("data:image/png;base64,AA==").is_none());
        assert!(catalog().len() > 100 && catalog().iter().all(|c| c["url"].as_str().unwrap().starts_with("data:image/svg+xml;base64,")));
    }

    #[test]
    fn uploads_are_checked_by_content_not_by_label() {
        let png = [&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..], &[0; 20]].concat();
        assert!(check(&uri("image/png", &png)).is_ok());
        assert!(check(&uri("image/jpeg", &png)).is_err(), "a PNG labelled JPEG");
        assert!(check(&uri("image/png", b"<svg xmlns='x'/>")).is_err());
        assert!(check(&uri("image/gif", b"GIF89a")).is_err(), "only PNG, JPEG, WebP and SVG");
        assert!(check(&uri("image/svg+xml", b"<svg xmlns='http://www.w3.org/2000/svg'><circle r='4'/></svg>")).is_ok());
        assert!(check(&uri("image/svg+xml", b"<svg><script>alert(1)</script></svg>")).is_err());
        assert!(check(&uri("image/svg+xml", b"<svg onload='x()'/>")).is_err());
        assert!(check(&uri("image/svg+xml", b"<svg><foreignObject/></svg>")).is_err());
        assert!(check(&uri("image/webp", b"RIFF\0\0\0\0WEBPVP8 ")).is_ok());
        assert!(check("data:image/png;base64,@@@").is_err() && check("data:image/png,rawbytes").is_err());
    }

    #[test]
    fn big_uploads_are_refused() {
        let mut big = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        big.resize(80_000, 0);
        assert!(check(&uri("image/png", &big)).is_err());
    }
}
