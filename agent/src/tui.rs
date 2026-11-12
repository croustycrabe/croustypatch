//! `croustypatch-portal`: the app center in a terminal (spec portail_app §4.2), for servers without a desktop.
//! Runs as an ordinary user; talks to the local agent's socket (see `portal.rs`).
//!
//! Keys: ↑/↓ choose an app · ←/→ change category · i install · u uninstall · r refresh · c clear notifications ·
//! q quit. Each app's colour says what is really on this machine (checked by the agent at every refresh); the
//! status area shows the operation in progress and a log of what happened (success, failure, information).
use crate::portal::{ask, Request};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap};
use ratatui::Frame;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

// CroustyLab Software, dark-theme tokens (terminals are mostly dark).
const BRICK: Color = Color::Rgb(0xd0, 0x78, 0x6b);
const SUCCESS: Color = Color::Rgb(0x4c, 0xc3, 0xae);
const WARNING: Color = Color::Rgb(0xf0, 0xb4, 0x29);
const DANGER: Color = Color::Rgb(0xff, 0x7d, 0x8f);
const INFO: Color = Color::Rgb(0x6c, 0xb1, 0xf0);
const MUTED: Color = Color::Rgb(0xa9, 0xb6, 0xbb);
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
const MAX_NOTES: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct AppItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub category: String,
    pub description: String,
    pub packages: Vec<String>,
    /// Command recipe (install, uninstall, check path) instead of packages.
    pub command: Option<(String, String, String)>,
    /// Per the server's last inventory of this host.
    pub installed: bool,
    /// Checked on this machine just now: (packages present, packages in the app).
    pub local: Option<(usize, usize)>,
    /// (installing?, status) while a request runs.
    pub pending: Option<(bool, String)>,
    /// A newer version than the one installed (apps built from the AUR): installing again updates it.
    pub update: Option<String>,
}

impl AppItem {
    pub fn from_json(v: &Value) -> Option<Self> {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let n = |p: &str| v.pointer(p).and_then(|x| x.as_u64()).map(|x| x as usize);
        Some(Self {
            id: v.get("id")?.as_str()?.to_string(),
            name: s("name"),
            version: s("version"),
            category: s("category"),
            description: s("description"),
            packages: v.get("packages").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default(),
            command: v.get("command").filter(|c| c.is_object()).map(|c| {
                let f = |k: &str| c.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                (f("install"), f("uninstall"), f("check"))
            }),
            installed: v.get("installed").and_then(|x| x.as_bool()).unwrap_or(false),
            local: n("/local/present").zip(n("/local/total")),
            pending: v.get("pending").filter(|p| !p.is_null()).map(|p| (p.get("install").and_then(|x| x.as_bool()).unwrap_or(true), p.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string())),
            update: (v.pointer("/local/update").and_then(|u| u.as_bool()) == Some(true)).then(|| v.pointer("/local/latest_version").and_then(|l| l.as_str()).unwrap_or("").to_string()),
        })
    }

    /// What is on the machine: the local check when the agent made one, else the server's inventory.
    pub fn presence(&self) -> Presence {
        match self.local {
            Some((p, t)) if t > 0 && p == t => Presence::Present,
            Some((p, t)) if p > 0 => Presence::Partial(p, t),
            Some(_) => Presence::Absent,
            None if self.installed => Presence::Present,
            None => Presence::Absent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Presence {
    Present,
    Partial(usize, usize),
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Info,
    Ok,
    Warn,
    Danger,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub tone: Tone,
    pub at: String,
    pub text: String,
}

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Refresh,
    Install(String),
    Uninstall(String),
    Quit,
}

/// Interface strings, French or English from $LANG.
pub struct Words {
    pub title: &'static str,
    pub all: &'static str,
    pub present: &'static str,
    pub partial: &'static str,
    pub available: &'static str,
    pub installing: &'static str,
    pub removing: &'static str,
    pub empty: &'static str,
    pub keys: &'static str,
    pub confirm_install: &'static str,
    pub confirm_remove: &'static str,
    pub yes_no: &'static str,
    pub sent: &'static str,
    pub packages: &'static str,
    pub missing: &'static str,
    pub command: &'static str,
    pub check: &'static str,
    pub categories: &'static str,
    pub apps: &'static str,
    pub status: &'static str,
    pub idle: &'static str,
    pub queued: &'static str,
    pub to_agent: &'static str,
    pub running: &'static str,
    pub installed_ok: &'static str,
    pub removed_ok: &'static str,
    pub failed: &'static str,
    pub cancelled: &'static str,
    pub opened: &'static str,
    pub update: &'static str,
    pub confirm_update: &'static str,
    pub updates: &'static str,
}

pub const FR: Words = Words {
    title: "Portail applicatif", all: "Toutes", present: "Présent", partial: "Partiel", available: "Disponible", installing: "Installation", removing: "Désinstallation",
    empty: "Aucune application proposée pour cette machine.", keys: "↑↓ choisir  ←→ catégorie  i installer  u désinstaller  r actualiser  c effacer  q quitter",
    confirm_install: "Installer", confirm_remove: "Désinstaller", yes_no: "o / n", sent: "demande envoyée", packages: "Paquets", missing: "manquants", command: "Commande", check: "Vérifié par",
    categories: "Catégories", apps: "Applications", status: "État et notifications", idle: "Aucune opération en cours.", queued: "en file d’attente", to_agent: "transmise à l’agent",
    running: "en cours", installed_ok: "installé", removed_ok: "désinstallé", failed: "échec", cancelled: "annulé", opened: "applications déjà présentes sur cette machine",
    update: "Mise à jour", confirm_update: "Mettre à jour", updates: "mise(s) à jour disponible(s)",
};
pub const EN: Words = Words {
    title: "App center", all: "All", present: "Present", partial: "Partial", available: "Available", installing: "Installing", removing: "Removing",
    empty: "No application offered for this machine.", keys: "↑↓ select  ←→ category  i install  u uninstall  r refresh  c clear  q quit",
    confirm_install: "Install", confirm_remove: "Uninstall", yes_no: "y / n", sent: "request sent", packages: "Packages", missing: "missing", command: "Command", check: "Checked by",
    categories: "Categories", apps: "Applications", status: "Status and notifications", idle: "No operation in progress.", queued: "queued", to_agent: "handed to the agent",
    running: "running", installed_ok: "installed", removed_ok: "uninstalled", failed: "failed", cancelled: "cancelled", opened: "applications already present on this machine",
    update: "Update", confirm_update: "Update", updates: "update(s) available",
};

pub struct State {
    pub host: String,
    pub apps: Vec<AppItem>,
    pub cat: usize,
    pub sel: usize,
    pub confirm: Option<(String, String, bool)>,
    /// Newest first.
    pub notes: VecDeque<Note>,
    /// Last status seen for each recent operation (task id), to notify each change once.
    seen: HashMap<String, String>,
    loaded: bool,
    pub tick: usize,
    pub w: &'static Words,
}

fn now_hms() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

impl State {
    pub fn new(w: &'static Words) -> Self {
        Self { host: String::new(), apps: vec![], cat: 0, sel: 0, confirm: None, notes: VecDeque::new(), seen: HashMap::new(), loaded: false, tick: 0, w }
    }

    pub fn note(&mut self, tone: Tone, text: impl Into<String>) {
        self.notes.push_front(Note { tone, at: now_hms(), text: text.into() });
        self.notes.truncate(MAX_NOTES);
    }

    pub fn categories(&self) -> Vec<String> {
        let mut c: Vec<String> = self.apps.iter().map(|a| a.category.clone()).filter(|c| !c.is_empty()).collect();
        c.sort();
        c.dedup();
        c.insert(0, self.w.all.to_string());
        c
    }

    pub fn visible(&self) -> Vec<&AppItem> {
        let cats = self.categories();
        let want = cats.get(self.cat).cloned().unwrap_or_default();
        self.apps.iter().filter(|a| self.cat == 0 || a.category == want).collect()
    }

    pub fn selected(&self) -> Option<&AppItem> {
        self.visible().get(self.sel).copied()
    }

    /// Take a fresh view from the agent, and turn what changed into notifications.
    pub fn load(&mut self, data: &Value) {
        self.host = data.pointer("/host/hostname").and_then(|h| h.as_str()).unwrap_or("").to_string();
        let keep = self.selected().map(|a| a.id.clone());
        self.apps = data.get("apps").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(AppItem::from_json).collect()).unwrap_or_default();
        self.cat = self.cat.min(self.categories().len().saturating_sub(1));
        self.sel = keep.and_then(|id| self.visible().iter().position(|a| a.id == id)).unwrap_or(self.sel).min(self.visible().len().saturating_sub(1));

        let recent = data.get("recent").and_then(|r| r.as_array()).cloned().unwrap_or_default();
        // Oldest first, so the log reads in order.
        for t in recent.iter().rev() {
            let (Some(id), Some(status)) = (t.get("task_id").and_then(|x| x.as_str()), t.get("status").and_then(|x| x.as_str())) else { continue };
            let before = self.seen.insert(id.to_string(), status.to_string());
            if !self.loaded || before.as_deref() == Some(status) {
                continue; // history from before the portal opened is not news
            }
            let name = t.get("app_name").and_then(|x| x.as_str()).unwrap_or("?");
            let install = t.get("install").and_then(|x| x.as_bool()).unwrap_or(true);
            let msg = t.get("message").and_then(|x| x.as_str()).unwrap_or("");
            let w = self.w;
            match status {
                "succeeded" => self.note(Tone::Ok, format!("{name} {}", if install { w.installed_ok } else { w.removed_ok })),
                "failed" | "expired" | "rolled_back" | "quarantined" => self.note(Tone::Danger, format!("{name}: {} — {msg}", w.failed)),
                "cancelled" | "rejected" | "blocked" => self.note(Tone::Warn, format!("{name}: {}", w.cancelled)),
                "sent" => self.note(Tone::Info, format!("{name}: {}", w.to_agent)),
                "running" => self.note(Tone::Info, format!("{name}: {} — {msg}", w.running)),
                _ => {}
            }
        }
        if !self.loaded {
            self.loaded = true;
            let present: Vec<String> = self.apps.iter().filter(|a| a.presence() == Presence::Present).map(|a| a.name.clone()).collect();
            if !present.is_empty() {
                let text = format!("{} {}: {}", present.len(), self.w.opened, present.join(", "));
                self.note(Tone::Info, text);
            }
            let updates: Vec<String> = self.apps.iter().filter_map(|a| a.update.as_ref().map(|v| format!("{} {v}", a.name))).collect();
            if !updates.is_empty() {
                let text = format!("{} {}: {}", updates.len(), self.w.updates, updates.join(", "));
                self.note(Tone::Warn, text);
            }
        }
    }

    pub fn busy(&self) -> bool {
        self.apps.iter().any(|a| a.pending.is_some())
    }

    /// The operation in progress: (app name, installing?, stage ratio, stage label).
    pub fn progress(&self) -> Option<(String, bool, f64, &'static str)> {
        let a = self.apps.iter().find(|a| a.pending.is_some())?;
        let (install, st) = a.pending.clone()?;
        let (ratio, label) = match st.as_str() {
            "queued" | "pending_approval" => (0.15, self.w.queued),
            "sent" => (0.4, self.w.to_agent),
            _ => (0.7, self.w.running),
        };
        Some((a.name.clone(), install, ratio, label))
    }

    pub fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> Option<Cmd> {
        if let Some((id, _, install)) = self.confirm.clone() {
            self.confirm = None;
            return matches!(code, KeyCode::Char('y' | 'o' | 'Y' | 'O') | KeyCode::Enter).then(|| if install { Cmd::Install(id) } else { Cmd::Uninstall(id) });
        }
        let n = self.visible().len();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Some(Cmd::Quit),
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => return Some(Cmd::Quit),
            KeyCode::Char('c') => self.notes.clear(),
            KeyCode::Down | KeyCode::Char('j') if n > 0 => self.sel = (self.sel + 1).min(n - 1),
            KeyCode::Up | KeyCode::Char('k') => self.sel = self.sel.saturating_sub(1),
            KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => {
                self.cat = (self.cat + 1) % self.categories().len().max(1);
                self.sel = 0;
            }
            KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                let len = self.categories().len().max(1);
                self.cat = (self.cat + len - 1) % len;
                self.sel = 0;
            }
            KeyCode::Char('r') => return Some(Cmd::Refresh),
            KeyCode::Char('i') | KeyCode::Enter => {
                if let Some(a) = self.selected().filter(|a| (a.presence() != Presence::Present || a.update.is_some()) && a.pending.is_none()) {
                    self.confirm = Some((a.id.clone(), a.name.clone(), true));
                }
            }
            KeyCode::Char('u') | KeyCode::Delete => {
                if let Some(a) = self.selected().filter(|a| a.presence() != Presence::Absent && a.pending.is_none()) {
                    self.confirm = Some((a.id.clone(), a.name.clone(), false));
                }
            }
            _ => {}
        }
        None
    }
}

fn state_cell<'a>(a: &AppItem, s: &State) -> Cell<'a> {
    let w = s.w;
    let spin = SPINNER[s.tick % SPINNER.len()];
    match (&a.pending, a.presence()) {
        (Some((true, _)), _) => Cell::from(format!("{spin} {}…", w.installing)).style(Style::new().fg(INFO)),
        (Some((false, _)), _) => Cell::from(format!("{spin} {}…", w.removing)).style(Style::new().fg(INFO)),
        (None, Presence::Present) if a.update.is_some() => Cell::from(format!("↑ {} {}", w.update, a.update.as_deref().unwrap_or(""))).style(Style::new().fg(WARNING).add_modifier(Modifier::BOLD)),
        (None, Presence::Present) => Cell::from(format!("● {}", w.present)).style(Style::new().fg(SUCCESS).add_modifier(Modifier::BOLD)),
        (None, Presence::Partial(p, t)) => Cell::from(format!("◑ {} {p}/{t}", w.partial)).style(Style::new().fg(WARNING)),
        (None, Presence::Absent) => Cell::from(format!("○ {}", w.available)).style(Style::new().fg(MUTED)),
    }
}

fn tone_style(t: Tone) -> (Color, &'static str) {
    match t {
        Tone::Ok => (SUCCESS, "✔"),
        Tone::Warn => (WARNING, "⚠"),
        Tone::Danger => (DANGER, "✖"),
        Tone::Info => (INFO, "ℹ"),
    }
}

pub fn render(f: &mut Frame, s: &State) {
    let area = f.area();
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(6), Constraint::Length(10), Constraint::Length(1)]).split(area);
    let title = Line::from(vec![
        Span::styled("[ ", Style::new().fg(BRICK).add_modifier(Modifier::BOLD)),
        Span::styled("CroustyPatch", Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(" ]", Style::new().fg(BRICK).add_modifier(Modifier::BOLD)),
        Span::styled(concat!(" v", env!("CARGO_PKG_VERSION")), Style::new().fg(MUTED)),
        Span::raw(format!("  {} · ", s.w.title)),
        Span::styled(s.host.clone(), Style::new().fg(MUTED)),
    ]);
    f.render_widget(Paragraph::new(title), rows[0]);

    // Categories | applications
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(24), Constraint::Min(30)]).split(rows[1]);
    let cats: Vec<ListItem> = s.categories().into_iter().map(ListItem::new).collect();
    let mut ls = ListState::default().with_selected(Some(s.cat));
    f.render_stateful_widget(List::new(cats).block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.w.categories))).highlight_style(Style::new().fg(BRICK).add_modifier(Modifier::BOLD)).highlight_symbol("› "), cols[0], &mut ls);
    let vis = s.visible();
    if vis.is_empty() {
        f.render_widget(Paragraph::new(s.w.empty).style(Style::new().fg(MUTED)).block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.w.apps))), cols[1]);
    } else {
        let rows_t: Vec<Row> = vis.iter().map(|a| Row::new(vec![Cell::from(a.name.clone()).style(Style::new().add_modifier(Modifier::BOLD)), Cell::from(a.version.clone()).style(Style::new().fg(MUTED)), state_cell(a, s)])).collect();
        let mut ts = TableState::default().with_selected(Some(s.sel));
        let t = Table::new(rows_t, [Constraint::Min(18), Constraint::Length(12), Constraint::Length(22)]).block(Block::default().borders(Borders::ALL).title(format!(" {} ", s.w.apps))).row_highlight_style(Style::new().bg(Color::Rgb(0x3a, 0x24, 0x20))).highlight_symbol("› ");
        f.render_stateful_widget(t, cols[1], &mut ts);
    }

    // Details | status and notifications
    let bottom = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(42), Constraint::Percentage(58)]).split(rows[2]);
    let detail = match s.selected() {
        Some(a) => {
            let mut l = vec![Line::from(Span::styled(a.name.clone(), Style::new().add_modifier(Modifier::BOLD))), Line::from(Span::raw(a.description.clone()))];
            match &a.command {
                Some((install, _, check)) => {
                    l.push(Line::from(vec![Span::styled(format!("{}: ", s.w.command), Style::new().fg(MUTED)), Span::raw(install.lines().next().unwrap_or("").to_string())]));
                    l.push(Line::from(vec![Span::styled(format!("{}: ", s.w.check), Style::new().fg(MUTED)), Span::raw(check.clone())]));
                }
                None => l.push(Line::from(vec![Span::styled(format!("{}: ", s.w.packages), Style::new().fg(MUTED)), Span::raw(a.packages.join(", "))])),
            }
            if let Presence::Partial(p, t) = a.presence() {
                l.push(Line::from(Span::styled(format!("{} {p}/{t} · {} {}", s.w.partial, t - p, s.w.missing), Style::new().fg(WARNING))));
            }
            l
        }
        None => vec![],
    };
    f.render_widget(Paragraph::new(detail).wrap(Wrap { trim: true }).block(Block::default().borders(Borders::ALL)), bottom[0]);

    let block = Block::default().borders(Borders::ALL).title(format!(" {} ", s.w.status)).border_style(Style::new().fg(if s.busy() { INFO } else { Color::Reset }));
    let inner = block.inner(bottom[1]);
    f.render_widget(block, bottom[1]);
    let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    match s.progress() {
        Some((name, install, ratio, stage)) => {
            let verb = if install { s.w.installing } else { s.w.removing };
            let g = Gauge::default().gauge_style(Style::new().fg(BRICK).bg(Color::Rgb(0x2f, 0x3b, 0x42))).ratio(ratio).label(format!("{} {verb} {name} · {stage}", SPINNER[s.tick % SPINNER.len()]));
            f.render_widget(g, parts[0]);
        }
        None => f.render_widget(Paragraph::new(Span::styled(s.w.idle, Style::new().fg(MUTED))), parts[0]),
    }
    let notes: Vec<Line> = s
        .notes
        .iter()
        .take(parts[1].height as usize)
        .map(|n| {
            let (c, icon) = tone_style(n.tone);
            Line::from(vec![Span::styled(format!("{} ", n.at), Style::new().fg(MUTED)), Span::styled(format!("{icon} "), Style::new().fg(c)), Span::styled(n.text.clone(), Style::new().fg(if n.tone == Tone::Danger { DANGER } else { Color::Reset }))])
        })
        .collect();
    f.render_widget(Paragraph::new(notes).wrap(Wrap { trim: true }), parts[1]);

    f.render_widget(Paragraph::new(Line::from(Span::styled(s.w.keys, Style::new().fg(MUTED)))), rows[3]);

    if let Some((id, name, install)) = &s.confirm {
        // A command app shows exactly what will run as root before the user says yes.
        let script = s.apps.iter().find(|a| &a.id == id).and_then(|a| a.command.as_ref()).map(|c| if *install { c.0.clone() } else { c.1.clone() });
        let extra: Vec<Line> = script.iter().flat_map(|sc| sc.lines().take(8).map(|l| Line::from(Span::styled(format!("  {l}"), Style::new().fg(MUTED)))).collect::<Vec<_>>()).collect();
        let r = centered(area, if extra.is_empty() { 50 } else { 76 }, 5 + extra.len() as u16 + u16::from(!extra.is_empty()));
        f.render_widget(Clear, r);
        let updating = *install && s.apps.iter().any(|a| &a.id == id && a.update.is_some());
        let verb = if updating { s.w.confirm_update } else if *install { s.w.confirm_install } else { s.w.confirm_remove };
        let mut lines = vec![Line::from(format!("{verb} {name} ?"))];
        if !extra.is_empty() {
            lines.push(Line::from(Span::styled(format!("{}:", s.w.command), Style::new().fg(MUTED))));
            lines.extend(extra);
        }
        lines.push(Line::from(Span::styled(s.w.yes_no, Style::new().fg(MUTED))));
        let p = Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::default().borders(Borders::ALL).border_style(Style::new().fg(if *install { BRICK } else { DANGER })));
        f.render_widget(p, r);
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

async fn refresh(socket: &str, s: &mut State) {
    match ask(socket, &Request::List).await {
        Ok(r) if r.ok => s.load(&r.data),
        Ok(r) => s.note(Tone::Danger, r.error),
        Err(e) => s.note(Tone::Danger, format!("{e:#}")),
    }
}

/// Run the TUI until the user quits.
pub async fn run(socket: &str) -> Result<()> {
    let fr = std::env::var("LC_ALL").or_else(|_| std::env::var("LANG")).unwrap_or_default().starts_with("fr");
    let mut s = State::new(if fr { &FR } else { &EN });
    refresh(socket, &mut s).await;
    let mut term = ratatui::init();
    let mut last = Instant::now();
    let res: Result<()> = async {
        loop {
            s.tick = s.tick.wrapping_add(1);
            term.draw(|f| render(f, &s))?;
            if event::poll(Duration::from_millis(150))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    match s.on_key(k.code, k.modifiers) {
                        Some(Cmd::Quit) => break,
                        Some(Cmd::Refresh) => {
                            refresh(socket, &mut s).await;
                            last = Instant::now();
                        }
                        Some(cmd @ (Cmd::Install(_) | Cmd::Uninstall(_))) => {
                            let (req, install) = match cmd { Cmd::Install(id) => (Request::Install { app_id: id }, true), Cmd::Uninstall(id) => (Request::Uninstall { app_id: id }, false), _ => unreachable!() };
                            let name = s.selected().map(|a| a.name.clone()).unwrap_or_default();
                            match ask(socket, &req).await {
                                Ok(r) if r.ok => {
                                    let verb = if install { s.w.confirm_install } else { s.w.confirm_remove };
                                    let text = format!("{verb} {name}: {}", s.w.sent);
                                    s.note(Tone::Info, text);
                                }
                                Ok(r) => s.note(Tone::Danger, format!("{name}: {}", r.error)),
                                Err(e) => s.note(Tone::Danger, format!("{e:#}")),
                            }
                            refresh(socket, &mut s).await;
                            last = Instant::now();
                        }
                        None => {}
                    }
                }
            }
            // Follow a running operation closely; otherwise refresh now and then.
            let every = if s.busy() { Duration::from_secs(2) } else { Duration::from_secs(15) };
            if last.elapsed() >= every {
                refresh(socket, &mut s).await;
                last = Instant::now();
            }
        }
        Ok(())
    }
    .await;
    ratatui::restore();
    res
}
