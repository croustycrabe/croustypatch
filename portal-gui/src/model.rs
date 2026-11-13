//! What the app center knows about the machine's applications, and how it is filtered and sorted. Pure (serde_json
//! only), so it is tested on its own, without a window.
use serde_json::Value;

/// What is on this machine. The agent's local check wins over the server's inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Present,
    Partial,
    Absent,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub status: String,
    pub install: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    pub install: String,
    pub uninstall: String,
    pub check: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub version: String,
    pub category: String,
    pub description: String,
    pub icon: String,
    /// An uploaded image (a `data:` URL) or an address; only a `data:` URL is shown.
    pub icon_url: String,
    pub packages: Vec<String>,
    pub command: Option<Command>,
    pub installed: bool,
    /// (present, total) packages or checks, when the agent looked.
    pub local: Option<(usize, usize)>,
    pub missing: Vec<String>,
    pub installed_version: String,
    /// The newer version the agent found at the app's source, if any.
    pub update: Option<String>,
    pub pending: Option<Pending>,
}

fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

impl App {
    pub fn from_json(v: &Value) -> Option<App> {
        let n = |p: &str| v.pointer(p).and_then(Value::as_u64).map(|x| x as usize);
        let list = |p: &str| v.pointer(p).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        let kind_command = v.get("command").is_some_and(Value::is_object);
        Some(App {
            id: v.get("id")?.as_str()?.to_string(),
            name: text(v, "name"),
            version: text(v, "version"),
            category: text(v, "category"),
            description: text(v, "description"),
            icon: text(v, "icon"),
            icon_url: text(v, "icon_url"),
            packages: list("/packages"),
            command: kind_command.then(|| Command { install: text(&v["command"], "install"), uninstall: text(&v["command"], "uninstall"), check: text(&v["command"], "check") }),
            installed: v.get("installed").and_then(Value::as_bool).unwrap_or(false),
            local: n("/local/present").zip(n("/local/total")),
            missing: list("/local/missing"),
            installed_version: v.pointer("/local/installed_version").and_then(Value::as_str).unwrap_or("").to_string(),
            update: (v.pointer("/local/update").and_then(Value::as_bool) == Some(true)).then(|| v.pointer("/local/latest_version").and_then(Value::as_str).unwrap_or("").to_string()),
            pending: v.get("pending").filter(|p| !p.is_null()).map(|p| Pending { status: text(p, "status"), install: p.get("install").and_then(Value::as_bool).unwrap_or(true) }),
        })
    }

    pub fn presence(&self) -> Presence {
        match self.local {
            Some((p, t)) if t > 0 && p == t => Presence::Present,
            Some((p, t)) if t > 0 && p > 0 => Presence::Partial,
            Some((_, t)) if t > 0 => Presence::Absent,
            _ if self.installed => Presence::Present,
            _ => Presence::Absent,
        }
    }

    /// Software installed by a command is not in the package manager's update list: the agent says when its source has
    /// a newer version. Only an app that is on the machine can be updated.
    pub fn needs_update(&self) -> bool {
        self.presence() == Presence::Present && self.update.is_some()
    }

    fn search_text(&self) -> String {
        format!("{} {} {}", self.name, self.description, self.packages.join(" ")).to_lowercase()
    }
}

/// Which applications the list shows, besides the category and the search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    /// Present, or partly there.
    Installed,
    NeedsUpdate,
    NotInstalled,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Installed, Filter::NeedsUpdate, Filter::NotInstalled];

    pub fn key(self) -> &'static str {
        match self {
            Filter::All => "f_all",
            Filter::Installed => "f_installed",
            Filter::NeedsUpdate => "f_updates",
            Filter::NotInstalled => "f_available",
        }
    }

    pub fn matches(self, a: &App) -> bool {
        match self {
            Filter::All => true,
            Filter::Installed => a.presence() != Presence::Absent,
            Filter::NeedsUpdate => a.needs_update(),
            Filter::NotInstalled => a.presence() == Presence::Absent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Name,
    /// Needs an update, then installed, then partly installed, then not installed; by name within each.
    Status,
}

impl Sort {
    pub fn key(self) -> &'static str {
        match self {
            Sort::Name => "sort_name",
            Sort::Status => "sort_status",
        }
    }
    pub fn other(self) -> Sort {
        match self {
            Sort::Name => Sort::Status,
            Sort::Status => Sort::Name,
        }
    }
}

fn rank(a: &App) -> u8 {
    if a.needs_update() {
        0
    } else {
        match a.presence() {
            Presence::Present => 1,
            Presence::Partial => 2,
            Presence::Absent => 3,
        }
    }
}

/// How the user narrowed the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub category: String,
    pub query: String,
    pub filter: Filter,
    pub sort: Sort,
}

impl Default for View {
    fn default() -> Self {
        View { category: String::new(), query: String::new(), filter: Filter::All, sort: Sort::Name }
    }
}

/// The applications to show, in order. The category and the search narrow first; the filter counts (see
/// [`filter_counts`]) are taken before the filter itself, so a chip says how many apps it would show.
pub fn visible<'a>(apps: &'a [App], v: &View) -> Vec<&'a App> {
    let mut out: Vec<&App> = narrowed(apps, v).into_iter().filter(|a| v.filter.matches(a)).collect();
    out.sort_by(|a, b| {
        let by_name = a.name.to_lowercase().cmp(&b.name.to_lowercase());
        match v.sort {
            Sort::Name => by_name,
            Sort::Status => rank(a).cmp(&rank(b)).then(by_name),
        }
    });
    out
}

fn narrowed<'a>(apps: &'a [App], v: &View) -> Vec<&'a App> {
    let q = v.query.trim().to_lowercase();
    apps.iter().filter(|a| (v.category.is_empty() || a.category == v.category) && (q.is_empty() || a.search_text().contains(&q))).collect()
}

/// How many applications each filter would show with the current category and search.
pub fn filter_counts(apps: &[App], v: &View) -> [usize; 4] {
    let n = narrowed(apps, v);
    Filter::ALL.map(|f| n.iter().filter(|a| f.matches(a)).count())
}

/// The categories, sorted, each with its number of applications.
pub fn categories(apps: &[App]) -> Vec<(String, usize)> {
    let mut names: Vec<String> = apps.iter().map(|a| a.category.clone()).filter(|c| !c.is_empty()).collect();
    names.sort_by_key(|c| c.to_lowercase());
    names.dedup();
    names.into_iter().map(|c| { let n = apps.iter().filter(|a| a.category == c).count(); (c, n) }).collect()
}

/// A pending system update, as the agent lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct Upgrade {
    pub name: String,
    pub version: String,
    pub candidate: String,
    pub security: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Updates {
    pub allowed: bool,
    pub pending: Vec<Upgrade>,
    pub checked_at: Option<String>,
    pub reboot_required: bool,
    pub open_status: Option<String>,
}

impl Updates {
    pub fn from_json(v: &Value) -> Updates {
        Updates {
            allowed: v.get("allowed").and_then(Value::as_bool).unwrap_or(false),
            pending: v.get("pending").and_then(Value::as_array).map(|a| a.iter().filter_map(|p| Some(Upgrade {
                name: p.get("name")?.as_str()?.to_string(),
                version: text(p, "version"),
                candidate: p.get("candidate").and_then(Value::as_str).unwrap_or("?").to_string(),
                security: p.get("is_security").and_then(Value::as_bool) == Some(true),
            })).collect()).unwrap_or_default(),
            checked_at: v.get("checked_at").and_then(Value::as_str).map(str::to_owned),
            reboot_required: v.get("reboot_required").and_then(Value::as_bool) == Some(true),
            open_status: v.pointer("/open_task/status").and_then(Value::as_str).map(str::to_owned),
        }
    }
}

/// Progress of a stage, in percent, for the bar.
pub fn stage_percent(status: &str) -> u32 {
    match status {
        "queued" => 20,
        "sent" => 45,
        "running" => 75,
        "soaking" => 90,
        _ => 60,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apps() -> Vec<App> {
        let v = json!([
            { "id": "a", "name": "htop", "category": "Outils", "packages": ["htop"], "installed": true },
            { "id": "b", "name": "Dev tools", "category": "Développement", "packages": ["git", "make"], "installed": false, "local": { "present": 1, "total": 2, "missing": ["make"] } },
            { "id": "c", "name": "ncdu", "category": "Outils", "packages": ["ncdu"], "installed": false },
            { "id": "d", "name": "Code", "category": "Développement", "command": { "install": "i", "uninstall": "u", "check": "c" }, "installed": true, "local": { "present": 1, "total": 1, "update": true, "latest_version": "2.0", "installed_version": "1.0" } },
            { "id": "e", "name": "Zed", "category": "Développement", "installed": false, "local": { "present": 0, "total": 1, "update": true, "latest_version": "9" } },
        ]);
        v.as_array().unwrap().iter().filter_map(App::from_json).collect()
    }
    fn names(a: &[&App]) -> Vec<String> {
        a.iter().map(|a| a.name.clone()).collect()
    }
    fn view(filter: Filter) -> View {
        View { filter, ..View::default() }
    }

    #[test]
    fn presence_prefers_the_local_check_and_falls_back_to_the_inventory() {
        let a = apps();
        assert_eq!(a[0].presence(), Presence::Present, "inventory only");
        assert_eq!(a[1].presence(), Presence::Partial);
        assert_eq!(a[2].presence(), Presence::Absent);
        assert_eq!(a[3].presence(), Presence::Present);
        assert_eq!(a[4].presence(), Presence::Absent, "the local check says it is not there");
    }

    #[test]
    fn the_filters_pick_installed_apps_updates_and_what_is_missing() {
        let a = apps();
        assert_eq!(names(&visible(&a, &view(Filter::All))), ["Code", "Dev tools", "htop", "ncdu", "Zed"]);
        assert_eq!(names(&visible(&a, &view(Filter::Installed))), ["Code", "Dev tools", "htop"], "partly installed counts");
        assert_eq!(names(&visible(&a, &view(Filter::NeedsUpdate))), ["Code"], "an app that is not installed has nothing to update");
        assert_eq!(names(&visible(&a, &view(Filter::NotInstalled))), ["ncdu", "Zed"]);
    }

    #[test]
    fn the_status_sort_puts_updates_first_then_installed_then_the_rest() {
        let a = apps();
        let v = View { sort: Sort::Status, ..View::default() };
        assert_eq!(names(&visible(&a, &v)), ["Code", "htop", "Dev tools", "ncdu", "Zed"]);
    }

    #[test]
    fn filters_combine_with_the_category_and_the_search_and_the_counts_follow() {
        let a = apps();
        let mut v = View { category: "Développement".into(), ..View::default() };
        assert_eq!(filter_counts(&a, &v), [3, 2, 1, 1]);
        v.filter = Filter::Installed;
        assert_eq!(names(&visible(&a, &v)), ["Code", "Dev tools"]);
        v.query = " MAKE ".into();
        assert_eq!(names(&visible(&a, &v)), ["Dev tools"], "the search looks in the package names too, any case");
        assert_eq!(filter_counts(&a, &v), [1, 1, 0, 0], "counts ignore the chosen filter but follow the search");
    }

    #[test]
    fn categories_are_sorted_with_their_counts() {
        assert_eq!(categories(&apps()), [("Développement".to_string(), 3), ("Outils".to_string(), 2)]);
    }

    #[test]
    fn an_app_is_read_from_the_agents_json() {
        let a = apps();
        assert_eq!(a[1].missing, ["make"]);
        assert_eq!(a[3].command.as_ref().unwrap().check, "c");
        assert_eq!(a[3].update.as_deref(), Some("2.0"));
        assert_eq!(a[3].installed_version, "1.0");
        assert!(App::from_json(&json!({ "name": "no id" })).is_none());
        let p = App::from_json(&json!({ "id": "x", "pending": { "status": "running", "install": false } })).unwrap();
        assert_eq!(p.pending, Some(Pending { status: "running".into(), install: false }));
    }

    #[test]
    fn updates_are_read_from_the_agents_json() {
        let u = Updates::from_json(&json!({ "allowed": true, "pending": [{ "name": "openssl", "version": "1", "candidate": "2", "is_security": true }, { "name": "less", "version": "3" }], "reboot_required": true, "open_task": { "status": "running" } }));
        assert!(u.allowed && u.reboot_required);
        assert_eq!(u.pending.len(), 2);
        assert!(u.pending[0].security && !u.pending[1].security);
        assert_eq!(u.pending[1].candidate, "?");
        assert_eq!(u.open_status.as_deref(), Some("running"));
        assert_eq!(stage_percent("soaking"), 90);
    }
}
