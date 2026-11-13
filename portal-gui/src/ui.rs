//! The app center window (GPUI). Everything it shows comes from the agent's socket through [`crate::backend::Op`];
//! the filtering and sorting are in [`crate::model`].
use crate::assets::icon_path;
use crate::backend::Op;
use crate::i18n::{tr, Lang};
use crate::model::{self, App, Filter, Presence, Updates, View};
use crate::settings::{self, Settings, ThemeChoice};
use crate::theme::{palette, Palette};
use base64::Engine;
use gpui::{
    div, img, prelude::*, px, relative, svg, AnyElement, App as GpuiApp, ClickEvent, Context, FocusHandle, Hsla, Image, ImageFormat, IntoElement, KeyDownEvent, MouseButton, SharedString,
    Window, WindowAppearance,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

const QUICK: Duration = Duration::from_millis(2500);
const SLOW: Duration = Duration::from_secs(15);
const MAX_NOTES: usize = 50;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Apps,
    Updates,
}

#[derive(Clone, Copy, PartialEq)]
enum Tone {
    Ok,
    Err,
    Warn,
    Info,
}

struct Note {
    tone: Tone,
    at: String,
    text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Install,
    Uninstall,
    Complete,
    Update,
}

impl Action {
    fn key(self) -> &'static str {
        match self {
            Action::Install => "install",
            Action::Uninstall => "uninstall",
            Action::Complete => "complete",
            Action::Update => "update",
        }
    }
}

/// What the confirmation dialog is about.
enum Confirm {
    App(Box<App>, Action),
    Upgrades(Vec<String>),
}

pub struct AppCenter {
    lang: Lang,
    theme: ThemeChoice,
    focus: FocusHandle,
    search_focus: FocusHandle,
    host: String,
    apps: Vec<App>,
    icons: HashMap<String, Arc<Image>>,
    updates: Option<Updates>,
    updates_error: String,
    tab: Tab,
    view: View,
    error: String,
    notes: Vec<Note>,
    seen: HashMap<String, String>,
    seen_updates: HashMap<String, String>,
    loaded: bool,
    updates_loaded: bool,
    busy: bool,
    refreshing: bool,
    /// A check for updates is running: the `checked_at` it started from, and when to stop waiting.
    checking: Option<(Option<String>, Instant)>,
    unchecked: HashSet<String>,
    confirm: Option<Confirm>,
    version: &'static str,
    last_refresh: Instant,
}

fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// A `data:image/...;base64,` URL as an image GPUI can draw (other addresses are not fetched: the window never
/// reaches out to the network on its own).
fn decode_icon(url: &str, icon: &str) -> Option<Arc<Image>> {
    let src = [url, icon].into_iter().find(|s| s.starts_with("data:image/"))?;
    let (head, data) = src.strip_prefix("data:")?.split_once(";base64,")?;
    let format = match head {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::Webp,
        "image/svg+xml" => ImageFormat::Svg,
        _ => return None,
    };
    let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).ok()?;
    (bytes.len() <= 2 * 1024 * 1024).then(|| Arc::new(Image::from_bytes(format, bytes)))
}

impl AppCenter {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let saved = settings::load();
        let locale = std::env::var("LC_ALL").or_else(|_| std::env::var("LC_MESSAGES")).or_else(|_| std::env::var("LANG")).unwrap_or_default();
        let this = AppCenter {
            lang: saved.lang.unwrap_or_else(|| Lang::from_locale(&locale)),
            theme: saved.theme.unwrap_or(ThemeChoice::System),
            focus: cx.focus_handle(),
            search_focus: cx.focus_handle(),
            host: String::new(),
            apps: vec![],
            icons: HashMap::new(),
            updates: None,
            updates_error: String::new(),
            tab: Tab::Apps,
            view: View::default(),
            error: String::new(),
            notes: vec![],
            seen: HashMap::new(),
            seen_updates: HashMap::new(),
            loaded: false,
            updates_loaded: false,
            busy: false,
            refreshing: false,
            checking: None,
            unchecked: HashSet::new(),
            confirm: None,
            version: env!("CARGO_PKG_VERSION"),
            last_refresh: Instant::now(),
        };
        window.focus(&this.focus);
        // Refresh when the window comes back to the front, and follow the system's light/dark.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh(cx);
            }
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify()).detach();
        // Quick while something runs, so progress and the outcome show up promptly; slow otherwise.
        cx.spawn(async move |this, cx| {
            while let Ok(wait) = this.update(cx, |s, _| s.poll_interval()) {
                cx.background_executor().timer(wait).await;
                if this.update(cx, |s, cx| s.refresh(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        this
    }

    fn poll_interval(&self) -> Duration {
        if self.apps.iter().any(|a| a.pending.is_some()) || self.updates.as_ref().is_some_and(|u| u.open_status.is_some()) || self.checking.is_some() {
            QUICK
        } else {
            SLOW
        }
    }

    fn t(&self, key: &str) -> String {
        tr(self.lang, key, &[])
    }

    fn tv(&self, key: &str, vars: &[(&str, String)]) -> String {
        tr(self.lang, key, vars)
    }

    fn note(&mut self, tone: Tone, text: String) {
        self.notes.insert(0, Note { tone, at: now_hms(), text });
        self.notes.truncate(MAX_NOTES);
    }

    // ---- talking to the agent

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        self.last_refresh = Instant::now();
        cx.spawn(async move |this, cx| {
            let (list, upd) = cx.background_spawn(async { (Op::List.run(), Op::Updates.run()) }).await;
            this.update(cx, |s, cx| {
                s.refreshing = false;
                s.apply_list(list);
                s.apply_updates(upd);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn apply_list(&mut self, list: Result<Value, String>) {
        match list {
            Ok(v) => {
                self.error.clear();
                self.host = v.pointer("/host/hostname").and_then(Value::as_str).unwrap_or("").to_string();
                self.apps = v.get("apps").and_then(Value::as_array).map(|a| a.iter().filter_map(App::from_json).collect()).unwrap_or_default();
                self.icons = self.apps.iter().filter_map(|a| Some((a.id.clone(), decode_icon(&a.icon_url, &a.icon)?))).collect();
                self.track_recent(v.get("recent"));
                if !self.loaded {
                    let here: Vec<String> = self.apps.iter().filter(|a| a.presence() == Presence::Present).map(|a| a.name.clone()).collect();
                    let text = if here.is_empty() { self.t("none_present") } else { self.tv("opened", &[("l", here.join(", "))]) };
                    self.note(Tone::Info, text);
                }
                self.loaded = true;
            }
            Err(e) => self.error = e,
        }
    }

    /// Report each finished operation once; operations from before the window opened are only remembered.
    fn track_recent(&mut self, recent: Option<&Value>) {
        for r in recent.and_then(Value::as_array).into_iter().flatten() {
            let (Some(id), Some(status)) = (r.get("task_id").and_then(Value::as_str), r.get("status").and_then(Value::as_str)) else { continue };
            let prev = self.seen.insert(id.to_string(), status.to_string());
            if !self.loaded || prev.as_deref() == Some(status) {
                continue;
            }
            let n = r.get("app_name").and_then(Value::as_str).unwrap_or("?").to_string();
            let msg = r.get("message").and_then(Value::as_str).unwrap_or("").to_string();
            let install = r.get("install").and_then(Value::as_bool) != Some(false);
            match status {
                "succeeded" => self.note(Tone::Ok, self.tv(if install { "ok_install" } else { "ok_uninstall" }, &[("n", n)])),
                "failed" | "blocked" | "rejected" => self.note(Tone::Err, self.tv("failed", &[("n", n), ("m", msg)])),
                "cancelled" | "expired" => self.note(Tone::Warn, self.tv("cancelled", &[("n", n)])),
                _ => {}
            }
        }
    }

    fn apply_updates(&mut self, upd: Result<Value, String>) {
        match upd {
            Ok(v) => {
                let u = Updates::from_json(&v);
                for r in v.get("recent").and_then(Value::as_array).into_iter().flatten() {
                    let (Some(id), Some(status)) = (r.get("task_id").and_then(Value::as_str), r.get("status").and_then(Value::as_str)) else { continue };
                    let prev = self.seen_updates.insert(id.to_string(), status.to_string());
                    if !self.updates_loaded || prev.as_deref() == Some(status) {
                        continue;
                    }
                    let msg = r.get("message").and_then(Value::as_str).unwrap_or("").to_string();
                    if status == "succeeded" {
                        self.note(Tone::Ok, self.t("ok_upd"));
                    } else if ["failed", "rolled_back", "blocked", "rejected"].contains(&status) {
                        self.note(Tone::Err, self.tv("fail_upd", &[("m", msg)]));
                    }
                }
                self.updates_loaded = true;
                if let Some((from, until)) = self.checking.clone() {
                    let moved = u.checked_at != from;
                    if moved || Instant::now() > until {
                        if moved {
                            self.note(Tone::Info, self.tv("check_done", &[("n", u.pending.len().to_string())]));
                        }
                        self.checking = None;
                    }
                }
                self.updates = Some(u);
                self.updates_error.clear();
            }
            Err(e) => {
                self.updates = None;
                self.updates_error = e;
            }
        }
    }

    /// Run one operation on the agent, then refresh. `ok` is the note shown when the agent accepted it.
    fn send(&mut self, op: Op, ok: String, fail_key: &'static str, name: String, cx: &mut Context<Self>) {
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { op.run() }).await;
            this.update(cx, |s, cx| {
                s.busy = false;
                match r {
                    Ok(_) => s.note(Tone::Info, ok),
                    Err(e) => s.note(Tone::Err, s.tv(fail_key, &[("n", name), ("m", e)])),
                }
                s.refreshing = false;
                s.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn start_check(&mut self, cx: &mut Context<Self>) {
        self.checking = Some((self.updates.as_ref().and_then(|u| u.checked_at.clone()), Instant::now() + Duration::from_secs(120)));
        self.note(Tone::Info, self.t("check_started"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async { Op::Check.run() }).await;
            this.update(cx, |s, cx| {
                if let Err(e) = r {
                    s.checking = None;
                    s.note(Tone::Err, e);
                }
                s.refreshing = false;
                s.refresh(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn confirmed(&mut self, cx: &mut Context<Self>) {
        match self.confirm.take() {
            Some(Confirm::App(a, action)) => {
                let (op, ok) = if action == Action::Uninstall { (Op::Uninstall(a.id.clone()), "sent") } else { (Op::Install(a.id.clone()), "sent") };
                let note = self.tv(ok, &[("n", a.name.clone())]);
                self.send(op, note, "failed", a.name.clone(), cx);
            }
            Some(Confirm::Upgrades(names)) => {
                let note = self.tv("upd_sent", &[("n", names.len().to_string())]);
                self.send(Op::Update(names), note, "fail_upd", String::new(), cx);
            }
            None => {}
        }
    }

    fn ask(&mut self, a: &App, action: Action) {
        self.confirm = Some(Confirm::App(Box::new(a.clone()), action));
    }

    fn save_settings(&self) {
        settings::save(&Settings { lang: Some(self.lang), theme: Some(self.theme) });
    }

    fn selected_updates(&self) -> Vec<String> {
        self.updates.as_ref().map(|u| u.pending.iter().map(|p| p.name.clone()).filter(|n| !self.unchecked.contains(n)).collect()).unwrap_or_default()
    }

    fn running(&self) -> Option<(String, String, u32)> {
        if let Some(a) = self.apps.iter().find(|a| a.pending.is_some()) {
            let p = a.pending.as_ref()?;
            return Some((self.tv(if p.install { "installing" } else { "removing" }, &[("n", a.name.clone())]), self.stage(&p.status), model::stage_percent(&p.status)));
        }
        let st = self.updates.as_ref()?.open_status.clone()?;
        Some((self.t("upd_running"), self.stage(&st), model::stage_percent(&st)))
    }

    fn stage(&self, status: &str) -> String {
        let key = format!("stage_{status}");
        let s = self.t(&key);
        if s == key { status.to_string() } else { s }
    }

    // ---- keyboard

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if self.confirm.is_some() {
            match k.key.as_str() {
                "escape" => self.confirm = None,
                "enter" => self.confirmed(cx),
                _ => return,
            }
            cx.notify();
            return;
        }
        let searching = self.search_focus.is_focused(window);
        match k.key.as_str() {
            "f5" => self.refresh(cx),
            "r" if m.control => self.refresh(cx),
            "escape" if searching => window.focus(&self.focus),
            "backspace" if searching => {
                self.view.query.pop();
            }
            "v" if m.control && searching => {
                if let Some(t) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    self.view.query.extend(t.chars().filter(|c| !c.is_control()));
                }
            }
            "/" if !searching && self.tab == Tab::Apps => {
                window.focus(&self.search_focus);
            }
            _ if searching && !m.control && !m.alt && !m.platform => match &k.key_char {
                Some(ch) if !ch.chars().any(char::is_control) => self.view.query.push_str(ch),
                _ => return,
            },
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

// ---- drawing

fn pill(p: &Palette, bg: Hsla, fg: Hsla, icon: Option<&str>, label: String) -> AnyElement {
    let mut d = div().flex().items_center().gap_1().px_2().h(px(20.)).rounded(px(10.)).bg(bg).text_color(fg).text_size(px(11.));
    if let Some(i) = icon {
        d = d.child(svg().path(icon_path(i)).size(px(12.)).text_color(fg).flex_none());
    }
    let _ = p;
    d.child(label).into_any_element()
}

fn icon(name: &str, size: f32, color: Hsla) -> impl IntoElement {
    svg().path(icon_path(name)).size(px(size)).text_color(color).flex_none()
}

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Primary,
    Outline,
    Danger,
    Ghost,
}

impl AppCenter {
    fn button(&self, p: &Palette, id: impl Into<SharedString>, look: Look, glyph: Option<&str>, label: String, disabled: bool, on_click: impl Fn(&mut Self, &ClickEvent, &mut Window, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> AnyElement {
        let (bg, fg, hover, border) = match look {
            Look::Primary => (p.primary, p.primary_fg, p.primary_hover, p.primary),
            Look::Danger => (p.danger_subtle, p.danger, p.danger_subtle, p.danger),
            Look::Outline => (p.surface, p.fg, p.hover, p.border_strong),
            Look::Ghost => (Hsla::transparent_black(), p.muted, p.hover, Hsla::transparent_black()),
        };
        let mut b = div().id(id.into()).flex().items_center().justify_center().gap_1p5().h(px(30.)).px_3().rounded(px(6.)).border_1().border_color(border).bg(bg).text_color(fg).text_size(px(13.));
        if let Some(g) = glyph {
            b = b.child(icon(g, 14., fg));
        }
        b = b.child(label);
        if disabled {
            b.opacity(0.5).into_any_element()
        } else {
            b.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(on_click)).into_any_element()
        }
    }

    fn app_tile(&self, p: &Palette, a: &App) -> AnyElement {
        let tile = div().size(px(40.)).flex().items_center().justify_center().rounded(px(8.)).bg(p.primary_subtle).flex_none();
        match self.icons.get(&a.id) {
            Some(image) => tile.child(img(image.clone()).size(px(32.))).into_any_element(),
            None => tile.child(icon(&a.icon, 22., p.primary_text)).into_any_element(),
        }
    }

    fn card(&self, p: &Palette, a: &App, locked: bool, cx: &mut Context<Self>) -> AnyElement {
        let presence = a.presence();
        let busy = a.pending.is_some();
        let dis = locked || self.busy;
        let badge = if busy {
            pill(p, p.info_subtle, p.info, Some("refresh"), self.t("working"))
        } else {
            match presence {
                Presence::Present => pill(p, p.success_subtle, p.success, Some("check"), self.t("present")),
                Presence::Partial => {
                    let (n, t) = a.local.unwrap_or((0, 0));
                    pill(p, p.warning_subtle, p.warning_text, Some("warn"), self.tv("partial", &[("p", n.to_string()), ("t", t.to_string())]))
                }
                Presence::Absent => pill(p, p.sunken, p.muted, None, self.t("available")),
            }
        };
        let mut actions = div().flex().gap_2().ml_auto();
        if !busy {
            let id = a.id.clone();
            if presence != Presence::Present {
                let action = if presence == Presence::Partial { Action::Complete } else { Action::Install };
                let app = a.clone();
                actions = actions.child(self.button(p, format!("i-{id}"), Look::Primary, Some("download"), self.t(action.key()), dis, move |s, _, _, cx| { s.ask(&app, action); cx.notify(); }, cx));
            }
            if a.needs_update() {
                let app = a.clone();
                actions = actions.child(self.button(p, format!("u-{id}"), Look::Primary, Some("refresh"), self.t("update"), dis, move |s, _, _, cx| { s.ask(&app, Action::Update); cx.notify(); }, cx));
            }
            if presence != Presence::Absent {
                let app = a.clone();
                actions = actions.child(self.button(p, format!("d-{id}"), Look::Outline, Some("trash"), self.t("uninstall"), dis, move |s, _, _, cx| { s.ask(&app, Action::Uninstall); cx.notify(); }, cx));
            }
        }
        let version = if a.version.is_empty() { a.installed_version.clone() } else { a.version.clone() };
        let sub = [a.category.clone(), version].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        let what = if a.command.is_some() { self.t("command_badge") } else { a.packages.join(" · ") };
        let mut foot = div().flex().flex_wrap().items_center().gap_2().child(badge);
        if !busy && a.needs_update() {
            foot = foot.child(pill(p, p.warning_subtle, p.warning_text, Some("warn"), self.tv("update_avail", &[("v", a.update.clone().unwrap_or_default())])));
        }
        div()
            .id(SharedString::from(format!("card-{}", a.id)))
            .w(px(318.))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(10.))
            .border_1()
            .border_color(if busy { p.info } else { p.border })
            .bg(p.surface)
            .child(div().flex().gap_3().items_center().child(self.app_tile(p, a)).child(div().flex().flex_col().min_w_0().child(div().font_weight(gpui::FontWeight::MEDIUM).truncate().child(a.name.clone())).child(div().text_size(px(12.)).text_color(p.muted).truncate().child(sub))))
            .child(div().h(px(36.)).overflow_hidden().text_size(px(13.)).text_color(p.muted).child(a.description.clone()))
            .child(div().text_size(px(11.)).text_color(p.disabled).truncate().child(what))
            .child(foot.child(actions))
            .into_any_element()
    }

    fn chip(&self, p: &Palette, id: String, label: String, count: Option<usize>, active: bool, on_click: impl Fn(&mut Self, &ClickEvent, &mut Window, &mut Context<Self>) + 'static, cx: &mut Context<Self>) -> AnyElement {
        let (bg, fg, border) = if active { (p.primary_subtle, p.primary_text, p.primary) } else { (p.surface, p.fg, p.border) };
        let hover = p.hover;
        let mut d = div().id(SharedString::from(id)).flex().items_center().gap_1p5().h(px(28.)).px_3().rounded(px(14.)).border_1().border_color(border).bg(bg).text_color(fg).text_size(px(13.)).cursor_pointer().child(label);
        if let Some(n) = count {
            d = d.child(div().text_size(px(11.)).text_color(if active { fg } else { p.muted }).child(n.to_string()));
        }
        d.hover(move |s| s.bg(hover)).on_click(cx.listener(on_click)).into_any_element()
    }

    fn header(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let upd_n = self.updates.as_ref().map_or(0, |u| u.pending.len());
        let tab = |s: &Self, id: &'static str, label: String, badge: usize, which: Tab, cx: &mut Context<Self>| {
            let active = s.tab == which;
            let mut d = div().id(id).flex().items_center().gap_2().h(px(32.)).px_3().rounded(px(6.)).text_size(px(14.)).cursor_pointer().child(label);
            d = if active { d.bg(p.primary_subtle).text_color(p.primary_text).font_weight(gpui::FontWeight::MEDIUM) } else { d.text_color(p.muted).hover(|s| s.bg(p.hover)) };
            if badge > 0 {
                d = d.child(div().px_1p5().rounded(px(8.)).bg(p.primary).text_color(p.primary_fg).text_size(px(11.)).child(badge.to_string()));
            }
            d.on_click(cx.listener(move |s, _, _, cx| { s.tab = which; cx.notify(); }))
        };
        let search = div()
            .id("search")
            .track_focus(&self.search_focus)
            .flex()
            .items_center()
            .gap_2()
            .w(px(200.))
            .h(px(32.))
            .px_2()
            .rounded(px(6.))
            .border_1()
            .border_color(p.border_strong)
            .bg(p.surface)
            .child(icon("search", 14., p.muted))
            .child(if self.view.query.is_empty() { div().text_color(p.disabled).text_size(px(13.)).child(self.t("search")) } else { div().text_size(px(13.)).truncate().child(self.view.query.clone()) })
            .on_click(cx.listener(|s, _, window, cx| { window.focus(&s.search_focus); cx.notify(); }));
        let theme_icon = match self.theme { ThemeChoice::System => "auto", ThemeChoice::Light => "sun", ThemeChoice::Dark => "moon" };
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(p.border)
            .bg(p.surface)
            .child(svg().path("logo.svg").w(px(40.)).h(px(26.)).text_color(p.primary).flex_none())
            .child(div().flex().flex_col().child(div().text_size(px(16.)).font_weight(gpui::FontWeight::BOLD).child(self.t("title"))).child(div().text_size(px(12.)).text_color(p.muted).child(format!("{}  v{}", if self.host.is_empty() { String::new() } else { self.tv("host", &[("h", self.host.clone())]) }, self.version))))
            .child(div().flex().gap_1().ml_4().child(tab(self, "tab-apps", self.t("tab_apps"), 0, Tab::Apps, cx)).child(tab(self, "tab-updates", self.t("tab_updates"), upd_n, Tab::Updates, cx)))
            .child(div().flex_1())
            .child(search)
            .child(self.button(p, "lang", Look::Outline, Some("translate"), self.lang.code().to_uppercase(), false, |s, _, _, cx| { s.lang = s.lang.other(); s.save_settings(); cx.notify(); }, cx))
            .child(self.button(p, "theme", Look::Outline, Some(theme_icon), self.t(self.theme.key()), false, |s, _, _, cx| { s.theme = s.theme.next(); s.save_settings(); cx.notify(); }, cx))
            .child(self.button(p, "refresh", Look::Outline, Some("refresh"), self.t("refresh"), false, |s, _, _, cx| { s.refresh(cx); }, cx))
            .into_any_element()
    }

    fn apps_view(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let cats = model::categories(&self.apps);
        let mut side = div().id("cats").flex().flex_col().gap_1().w(px(200.)).flex_none().p_3().border_r_1().border_color(p.border).overflow_y_scroll();
        let row = |s: &Self, key: String, label: String, n: usize, value: String, cx: &mut Context<Self>| {
            let active = s.view.category == value;
            let mut d = div().id(SharedString::from(key)).flex().items_center().justify_between().h(px(32.)).px_3().rounded(px(6.)).text_size(px(14.)).cursor_pointer().child(div().truncate().child(label)).child(div().text_size(px(12.)).text_color(p.muted).child(n.to_string()));
            d = if active { d.bg(p.primary_subtle).text_color(p.primary_text) } else { d.hover(|s| s.bg(p.hover)) };
            d.on_click(cx.listener(move |s, _, _, cx| { s.view.category = value.clone(); cx.notify(); }))
        };
        side = side.child(row(self, "cat-all".into(), self.t("all"), self.apps.len(), String::new(), cx));
        for (c, n) in &cats {
            side = side.child(row(self, format!("cat-{c}"), c.clone(), *n, c.clone(), cx));
        }

        // The category must still exist (it can vanish after a refresh).
        let counts = model::filter_counts(&self.apps, &self.view);
        let mut bar = div().flex().flex_wrap().items_center().gap_2().child(div().text_size(px(12.)).text_color(p.muted).child(self.t("filter")));
        for (i, f) in Filter::ALL.into_iter().enumerate() {
            bar = bar.child(self.chip(p, format!("f-{i}"), self.t(f.key()), Some(counts[i]), self.view.filter == f, move |s, _, _, cx| { s.view.filter = f; cx.notify(); }, cx));
        }
        bar = bar.child(div().ml_auto().flex().items_center().gap_2().child(div().text_size(px(12.)).text_color(p.muted).child(self.t("sort"))).child(self.chip(p, "sort".into(), self.t(self.view.sort.key()), None, false, |s, _, _, cx| { s.view.sort = s.view.sort.other(); cx.notify(); }, cx)));

        let shown = model::visible(&self.apps, &self.view);
        let locked = self.apps.iter().any(|a| a.pending.is_some()); // one operation at a time per machine
        let mut grid = div().id("grid").w_full().flex().flex_wrap().gap_3().content_start().overflow_y_scroll().flex_1();
        for a in &shown {
            grid = grid.child(self.card(p, a, locked, cx));
        }
        let mut main = div().flex().flex_col().flex_1().min_w_0().gap_3().p_4().child(bar);
        if !self.error.is_empty() {
            let text = if self.error.contains("unreachable") { self.tv("unreachable", &[("e", self.error.clone())]) } else { self.error.clone() };
            main = main.child(div().flex().items_center().gap_2().p_3().rounded(px(8.)).bg(p.danger_subtle).text_color(p.danger).text_size(px(13.)).child(icon("x", 16., p.danger)).child(text));
        }
        main = main.child(grid);
        if self.loaded && shown.is_empty() {
            let key = if self.apps.is_empty() { "empty" } else if self.view.filter != Filter::All && self.view.query.is_empty() && self.view.category.is_empty() { "empty_filter" } else { "no_match" };
            main = main.child(div().p_6().text_color(p.muted).child(self.t(key)));
        }
        div().flex().flex_1().min_h_0().child(side).child(main).into_any_element()
    }

    fn updates_view(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let u = self.updates.clone();
        let pending = u.as_ref().map(|u| u.pending.clone()).unwrap_or_default();
        let sec = pending.iter().filter(|x| x.security).count();
        let busy = u.as_ref().is_some_and(|u| u.open_status.is_some());
        let checked = u.as_ref().and_then(|u| u.checked_at.clone()).and_then(|d| chrono::DateTime::parse_from_rfc3339(&d).ok()).map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string());
        let sub = match &u {
            None => self.tv("upd_unavailable", &[("e", self.updates_error.clone())]),
            Some(_) => [
                if pending.is_empty() { self.t("upd_none") } else { self.tv("upd_sub", &[("n", pending.len().to_string()), ("sec", if sec > 0 { self.tv("upd_sec", &[("s", sec.to_string())]) } else { String::new() })]) },
                checked.map(|d| self.tv("upd_checked", &[("d", d)])).unwrap_or_default(),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        };
        let chosen = self.selected_updates();
        let allowed = u.as_ref().is_some_and(|u| u.allowed);
        let mut head = div().flex().items_center().gap_3().child(div().flex().flex_col().child(div().text_size(px(16.)).font_weight(gpui::FontWeight::BOLD).child(self.t("upd_title"))).child(div().text_size(px(13.)).text_color(p.muted).child(sub)));
        head = head.child(div().flex_1());
        head = head.child(self.button(p, "upd-check", Look::Outline, Some("refresh"), self.t(if self.checking.is_some() { "upd_checking" } else { "upd_check" }), self.checking.is_some() || u.is_none(), |s, _, _, cx| s.start_check(cx), cx));
        if allowed {
            let n = chosen.len();
            head = head.child(self.button(p, "upd-go", Look::Primary, Some("download"), self.tv("upd_go", &[("n", n.to_string())]), busy || n == 0 || self.busy, move |s, _, _, cx| { s.confirm = Some(Confirm::Upgrades(s.selected_updates())); cx.notify(); }, cx));
        }
        let mut col = div().flex().flex_col().flex_1().min_h_0().gap_3().p_4().child(head);
        let notice = |bg: Hsla, fg: Hsla, text: String| div().p_3().rounded(px(8.)).bg(bg).text_color(fg).text_size(px(13.)).child(text);
        if u.as_ref().is_some_and(|u| u.reboot_required) {
            col = col.child(notice(p.warning_subtle, p.warning_text, self.t("upd_reboot")));
        }
        if u.is_some() && !allowed {
            col = col.child(notice(p.info_subtle, p.info, self.t("upd_managed")));
        }
        let mut list = div().id("upd-list").flex().flex_col().flex_1().overflow_y_scroll().rounded(px(8.)).border_1().border_color(p.border).bg(p.surface);
        for x in &pending {
            let on = !self.unchecked.contains(&x.name);
            let name = x.name.clone();
            let mut checkbox = div().size(px(18.)).flex().items_center().justify_center().rounded(px(4.)).border_1().border_color(if on { p.primary } else { p.border_strong }).bg(if on { p.primary } else { p.surface });
            if on {
                checkbox = checkbox.child(icon("check", 12., p.primary_fg));
            }
            let mut row = div().id(SharedString::from(format!("up-{}", x.name))).flex().items_center().gap_3().px_3().h(px(38.)).border_b_1().border_color(p.border).child(checkbox).child(div().flex_1().truncate().child(x.name.clone())).child(div().text_size(px(12.)).text_color(p.muted).child(format!("{} → {}", x.version, x.candidate)));
            if x.security {
                row = row.child(pill(p, p.warning_subtle, p.warning_text, Some("warn"), self.t("upd_security")));
            }
            if allowed && !busy {
                row = row.cursor_pointer().hover(|s| s.bg(p.hover)).on_click(cx.listener(move |s, _, _, cx| {
                    if !s.unchecked.remove(&name) {
                        s.unchecked.insert(name.clone());
                    }
                    cx.notify();
                }));
            }
            list = list.child(row);
        }
        if u.is_some() && pending.is_empty() {
            list = list.child(div().p_6().text_color(p.muted).child(self.t("upd_none")));
        }
        col.child(list).into_any_element()
    }

    fn status(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut log = div().id("log").flex().flex_col().gap_0p5().h(px(92.)).overflow_y_scroll().text_size(px(12.));
        for n in &self.notes {
            let (glyph, color) = match n.tone {
                Tone::Ok => ("check", p.success),
                Tone::Err => ("x", p.danger),
                Tone::Warn => ("warn", p.warning_text),
                Tone::Info => ("info", p.info),
            };
            log = log.child(div().flex().gap_2().items_center().child(div().text_color(p.disabled).child(n.at.clone())).child(icon(glyph, 12., color)).child(div().child(n.text.clone())));
        }
        let mut col = div().flex().flex_col().gap_2().px_4().py_3().border_t_1().border_color(p.border).bg(p.surface);
        col = col.child(div().flex().items_center().child(div().text_size(px(13.)).font_weight(gpui::FontWeight::MEDIUM).child(self.t("status"))).child(div().flex_1()).child(self.button(p, "clear", Look::Ghost, Some("broom"), self.t("clear"), false, |s, _, _, cx| { s.notes.clear(); cx.notify(); }, cx)));
        match self.running() {
            Some((text, stage, pct)) => {
                col = col.child(div().flex().justify_between().text_size(px(13.)).child(text).child(div().text_color(p.muted).child(stage)));
                col = col.child(div().h(px(6.)).w_full().rounded(px(3.)).bg(p.sunken).child(div().h_full().rounded(px(3.)).bg(p.primary).w(relative(pct as f32 / 100.))));
            }
            None => col = col.child(div().text_size(px(13.)).text_color(p.muted).child(self.t("idle"))),
        }
        col.child(log).into_any_element()
    }

    fn dialog(&self, p: &Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let confirm = self.confirm.as_ref()?;
        let mut body = div().flex().flex_col().gap_2().text_size(px(13.));
        let pre = |text: String| div().p_2().rounded(px(6.)).bg(p.sunken).text_size(px(12.)).font_family("monospace").child(text);
        let (title, yes, danger) = match confirm {
            Confirm::App(a, action) => {
                if a.command.is_some() && *action != Action::Complete {
                    let c = a.command.as_ref()?;
                    body = body.child(self.t("command")).child(pre(if *action == Action::Uninstall { c.uninstall.clone() } else { c.install.clone() }));
                    if *action != Action::Uninstall {
                        body = body.child(div().text_color(p.muted).child(self.tv("check", &[("p", c.check.clone())])));
                    }
                } else {
                    body = body.child(self.t("packages")).child(pre(a.packages.join(" ")));
                    if *action == Action::Complete && !a.missing.is_empty() {
                        body = body.child(div().text_color(p.muted).child(self.tv("missing", &[("m", a.missing.join(", "))])));
                    }
                }
                (self.tv(&format!("confirm_{}", action.key()), &[("n", a.name.clone())]), self.t(action.key()), *action == Action::Uninstall)
            }
            Confirm::Upgrades(names) => {
                body = body.child(self.t("upd_body")).child(pre(names.join(" ")));
                (self.tv("confirm_upd", &[("n", names.len().to_string())]), self.tv("upd_go", &[("n", names.len().to_string())]), false)
            }
        };
        let card = div()
            .id("dialog")
            .flex()
            .flex_col()
            .gap_3()
            .w(px(480.))
            .max_h(relative(0.8))
            .p_5()
            .rounded(px(10.))
            .border_1()
            .border_color(p.border_strong)
            .bg(p.surface)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(16.)).font_weight(gpui::FontWeight::BOLD).child(title))
            .child(body)
            .child(div().flex().justify_end().gap_2().child(self.button(p, "no", Look::Outline, None, self.t("cancel"), false, |s, _, _, cx| { s.confirm = None; cx.notify(); }, cx)).child(self.button(p, "yes", if danger { Look::Danger } else { Look::Primary }, None, yes, false, |s, _, _, cx| { s.confirmed(cx); cx.notify(); }, cx)));
        Some(div().id("overlay").absolute().inset_0().flex().items_center().justify_center().bg(p.overlay).on_mouse_down(MouseButton::Left, cx.listener(|s, _, _, cx| { s.confirm = None; cx.notify(); })).child(card).into_any_element())
    }
}

impl Render for AppCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dark = match self.theme {
            ThemeChoice::Dark => true,
            ThemeChoice::Light => false,
            ThemeChoice::System => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
        };
        let p = palette(dark);
        let body = if self.tab == Tab::Apps { self.apps_view(&p, cx) } else { self.updates_view(&p, cx) };
        let mut root = div()
            .id("root")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|s, ev: &KeyDownEvent, window, cx| s.on_key(ev, window, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(p.bg)
            .text_color(p.fg)
            .text_size(px(14.))
            .child(self.header(&p, cx))
            .child(body)
            .child(self.status(&p, cx));
        if let Some(d) = self.dialog(&p, cx) {
            root = root.child(d);
        }
        root
    }
}

/// Opens the window.
pub fn open(cx: &mut GpuiApp) {
    use gpui::{point, size, Bounds, TitlebarOptions, WindowBounds, WindowOptions};
    let bounds = Bounds::centered(None, size(px(1120.), px(740.)), cx);
    let _ = point(px(0.), px(0.));
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some("CroustyPatch".into()), ..Default::default() }),
            window_min_size: Some(size(px(720.), px(480.))),
            app_id: Some("croustypatch-portal".into()),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| AppCenter::new(window, cx));
            view.update(cx, |s, cx| s.refresh(cx));
            view
        },
    );
    if opened.is_err() {
        eprintln!("cannot open the CroustyPatch app center window (no display?)");
        cx.quit();
    }
    cx.activate(true);
}
