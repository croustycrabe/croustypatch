//! The language and theme the user chose, kept on this machine (`$XDG_CONFIG_HOME/croustypatch/portal-gui.conf`, two
//! `key=value` lines) until they choose again. Nothing is stored until a choice is made.
use crate::i18n::Lang;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeChoice {
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "theme_system",
            ThemeChoice::Light => "theme_light",
            ThemeChoice::Dark => "theme_dark",
        }
    }
    pub fn next(self) -> ThemeChoice {
        match self {
            ThemeChoice::System => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::System,
        }
    }
    fn code(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }
    fn from_code(c: &str) -> Option<ThemeChoice> {
        [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark].into_iter().find(|t| t.code() == c)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Settings {
    pub lang: Option<Lang>,
    pub theme: Option<ThemeChoice>,
}

pub fn parse(text: &str) -> Settings {
    let mut s = Settings::default();
    for line in text.lines() {
        match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
            Some(("lang", v)) => s.lang = Lang::from_code(v),
            Some(("theme", v)) => s.theme = ThemeChoice::from_code(v),
            _ => {}
        }
    }
    s
}

pub fn render(s: &Settings) -> String {
    format!("lang={}\ntheme={}\n", s.lang.map_or("", Lang::code), s.theme.map_or("", ThemeChoice::code))
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("croustypatch").join("portal-gui.conf"))
}

pub fn load() -> Settings {
    path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default()
}

/// Best effort: a read-only home only means the choice lasts for this session.
pub fn save(s: &Settings) {
    if let Some(p) = path() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, render(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_choice_survives_a_save_and_a_load_and_junk_is_ignored() {
        let s = Settings { lang: Some(Lang::Fr), theme: Some(ThemeChoice::Dark) };
        assert_eq!(parse(&render(&s)), s);
        assert_eq!(parse("lang=klingon\ntheme=\nwhat\n"), Settings::default());
        assert_eq!(parse(""), Settings::default());
        assert_eq!(ThemeChoice::Dark.next(), ThemeChoice::System);
    }
}
