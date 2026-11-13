//! Docker migration console: prebuilt command lines for moving a container to another image when a new tag is not
//! enough (a database's major version: the new image cannot open the old data directory). They are drafts for a
//! person to read and edit, then run from the Docker page (`POST /api/v1/docker/shell`); nothing here runs by itself.
//!
//! Every template starts with a backup, never deletes anything (the old data directory and the compose file are
//! renamed with a `.pre-upgrade` suffix) and comes with the steps that put them back. Names that end up in a command
//! are checked first: a container, image or tag with a character a shell could read is refused.
use croustypatch_common::protocol::ShellStep;
use serde::Serialize;

pub const BACKUP_DIR: &str = "/var/backups/croustypatch";

/// What a template is built from: one container and the image it moves to.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// Container name, without the `docker:` prefix.
    pub container: String,
    /// The image the container runs (`postgres:17-alpine`).
    pub image: String,
    /// The tag it moves to (`18-alpine`).
    pub to_tag: String,
    pub project: Option<String>,
    pub service: Option<String>,
    pub compose_dir: Option<String>,
    pub compose_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Template {
    pub id: String,
    pub title: String,
    pub description: String,
    pub steps: Vec<ShellStep>,
    /// What puts things back when the migration went wrong (also editable, run the same way).
    pub rollback: Vec<ShellStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Postgres,
    Mysql,
    Mongo,
    Redis,
    Other,
}

/// The kind of software an image is, from the words of its repository name.
pub fn family(image: &str) -> Family {
    let repo = image.split('@').next().unwrap_or(image);
    let repo = match repo.rfind(':') {
        Some(i) if repo.rfind('/').is_none_or(|s| i > s) => &repo[..i],
        _ => repo,
    };
    let repo = repo.to_ascii_lowercase();
    let words: Vec<&str> = repo.split(|c: char| !c.is_ascii_alphanumeric()).collect();
    let has = |names: &[&str]| words.iter().any(|w| names.contains(w));
    if has(&["postgres", "postgresql", "postgis", "pgvector", "pgvecto", "vectorchord", "timescale", "timescaledb"]) {
        Family::Postgres
    } else if has(&["mysql", "mariadb", "percona"]) {
        Family::Mysql
    } else if has(&["mongo", "mongodb"]) {
        Family::Mongo
    } else if has(&["redis", "valkey", "keydb", "dragonfly"]) {
        Family::Redis
    } else {
        Family::Other
    }
}

/// Container names as Docker allows them.
pub fn valid_container(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// An image reference without anything a shell or sed would read.
pub fn valid_image(s: &str) -> bool {
    !s.is_empty() && s.len() <= 300 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '+' | '-'))
}

pub fn valid_tag(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// `postgres:17-alpine` and `18-alpine` give `postgres:18-alpine`; a registry port is not a tag.
pub fn with_tag(image: &str, tag: &str) -> String {
    let base = image.split('@').next().unwrap_or(image);
    match base.rfind(':') {
        Some(i) if base.rfind('/').is_none_or(|s| i > s) => format!("{}:{tag}", &base[..i]),
        _ => format!("{base}:{tag}"),
    }
}

/// Inside single quotes for a shell.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn sed_pattern(s: &str) -> String {
    s.chars().flat_map(|c| if matches!(c, '\\' | '/' | '.' | '*' | '[' | ']' | '^' | '$') { vec!['\\', c] } else { vec![c] }).collect()
}

fn sed_replacement(s: &str) -> String {
    s.chars().flat_map(|c| if matches!(c, '\\' | '&' | '/') { vec!['\\', c] } else { vec![c] }).collect()
}

fn step(title: &str, command: impl Into<String>) -> ShellStep {
    ShellStep { title: title.into(), command: command.into() }
}

fn compose_cmd(ctx: &Ctx, tail: &str) -> Option<String> {
    if ctx.compose_files.is_empty() {
        return None;
    }
    let mut s = String::from("docker compose");
    if let Some(d) = &ctx.compose_dir {
        s += &format!(" --project-directory {}", sq(d));
    }
    for f in &ctx.compose_files {
        s += &format!(" -f {}", sq(f));
    }
    if let Some(p) = &ctx.project {
        s += &format!(" -p {}", sq(p));
    }
    Some(format!("{s} {tail}"))
}

/// What differs between the families of databases.
struct Spec {
    name: &'static str,
    /// Writes the backup to the file given as `$DUMP` and checks it is not empty.
    dump: String,
    extension: &'static str,
    /// Container paths of the data directory (the first mount that matches is moved aside); empty: data stays.
    data: &'static [&'static str],
    ready: String,
    /// Feeds `$DUMP` back (None: the data stays in place and nothing is restored).
    restore: Option<String>,
    check: String,
}

fn spec(ctx: &Ctx, family: Family) -> Option<Spec> {
    let c = &ctx.container;
    Some(match family {
        Family::Postgres => Spec {
            name: "PostgreSQL",
            dump: format!("docker exec {c} sh -c 'pg_dumpall -U \"${{POSTGRES_USER:-postgres}}\"' > \"$DUMP\" && test -s \"$DUMP\" && ls -lh \"$DUMP\""),
            extension: "sql",
            data: &["/var/lib/postgresql/data", "/var/lib/postgresql"],
            ready: format!("for i in $(seq 1 60); do docker exec {c} pg_isready -q && break; sleep 2; done; docker exec {c} pg_isready"),
            restore: Some(format!("docker exec -i {c} sh -c 'psql -U \"${{POSTGRES_USER:-postgres}}\" -d postgres -q' < \"$DUMP\"")),
            check: format!("docker exec {c} sh -c 'psql -U \"${{POSTGRES_USER:-postgres}}\" -d postgres -c \"\\l\"'"),
        },
        Family::Mysql => {
            let pw = "-uroot -p\"${MYSQL_ROOT_PASSWORD:-$MARIADB_ROOT_PASSWORD}\"";
            Spec {
                name: "MySQL / MariaDB",
                dump: format!("docker exec {c} sh -c 'D=$(command -v mariadb-dump || command -v mysqldump); \"$D\" --all-databases --single-transaction --routines --events {pw}' > \"$DUMP\" && test -s \"$DUMP\" && ls -lh \"$DUMP\""),
                extension: "sql",
                data: &["/var/lib/mysql"],
                ready: format!("for i in $(seq 1 60); do docker exec {c} sh -c 'M=$(command -v mariadb-admin || command -v mysqladmin); \"$M\" ping {pw} --silent' && break; sleep 2; done"),
                restore: Some(format!("docker exec -i {c} sh -c 'M=$(command -v mariadb || command -v mysql); \"$M\" {pw}' < \"$DUMP\"")),
                check: format!("docker exec {c} sh -c 'M=$(command -v mariadb || command -v mysql); \"$M\" {pw} -e \"SHOW DATABASES\"'"),
            }
        }
        Family::Mongo => {
            let auth = "${MONGO_INITDB_ROOT_USERNAME:+-u \"$MONGO_INITDB_ROOT_USERNAME\" -p \"$MONGO_INITDB_ROOT_PASSWORD\" --authenticationDatabase admin}";
            Spec {
                name: "MongoDB",
                dump: format!("docker exec {c} sh -c 'mongodump --archive --gzip {auth}' > \"$DUMP\" && test -s \"$DUMP\" && ls -lh \"$DUMP\""),
                extension: "archive.gz",
                data: &["/data/db"],
                ready: format!("for i in $(seq 1 60); do docker exec {c} mongosh --quiet --eval 'db.runCommand({{ping:1}}).ok' | grep -q 1 && break; sleep 2; done"),
                restore: Some(format!("docker exec -i {c} sh -c 'mongorestore --archive --gzip {auth}' < \"$DUMP\"")),
                check: format!("docker exec {c} mongosh --quiet --eval 'db.adminCommand({{listDatabases:1}}).databases.map(d => d.name)'"),
            }
        }
        Family::Redis => Spec {
            name: "Redis / Valkey",
            dump: format!("docker exec {c} sh -c 'c=$(command -v valkey-cli || command -v redis-cli); \"$c\" SAVE' && docker cp {c}:/data/dump.rdb \"$DUMP\" && ls -lh \"$DUMP\""),
            extension: "rdb",
            data: &[],
            ready: format!("for i in $(seq 1 30); do docker exec {c} sh -c 'c=$(command -v valkey-cli || command -v redis-cli); \"$c\" PING' | grep -q PONG && break; sleep 2; done"),
            restore: None,
            check: format!("docker exec {c} sh -c 'c=$(command -v valkey-cli || command -v redis-cli); \"$c\" DBSIZE'"),
        },
        Family::Other => return None,
    })
}

fn dump_path(ctx: &Ctx, ext: &str) -> String {
    format!("{BACKUP_DIR}/{}-pre-upgrade.{ext}", ctx.container)
}

/// Finds the host path of the container's data mount (the first destination that matches) in `$SRC`.
fn find_src(ctx: &Ctx, dests: &[&str]) -> String {
    let cond = dests.iter().map(|d| format!("(eq .Destination \"{d}\")")).collect::<Vec<_>>().join(" ");
    let cond = if dests.len() > 1 { format!("or {cond}") } else { cond };
    format!(
        "SRC=$(docker inspect -f '{{{{range .Mounts}}}}{{{{if {cond}}}}}{{{{.Source}}}}{{{{end}}}}{{{{end}}}}' {c}); [ -n \"$SRC\" ] || {{ echo \"no data mount ({list}) found on {c}: do this step by hand\"; exit 1; }}",
        c = ctx.container,
        list = dests.join(" or ")
    )
}

fn retag_step(ctx: &Ctx, new_image: &str) -> Option<ShellStep> {
    if ctx.compose_files.is_empty() {
        return None;
    }
    let files = ctx.compose_files.iter().map(|f| sq(f)).collect::<Vec<_>>().join(" ");
    Some(step(
        "Change the image in the compose file (a copy is kept next to it)",
        format!(
            "DONE=0; for F in {files}; do if grep -qF {old} \"$F\"; then cp -p \"$F\" \"$F.pre-upgrade\" && sed -i 's|{pat}|{rep}|' \"$F\" && echo \"$F: {old_plain} -> {new}\" && DONE=1; fi; done; [ \"$DONE\" = 1 ] || {{ echo \"{old_plain} is not written in the compose file(s) (a variable?): change the image by hand, then run the next steps\"; exit 1; }}",
            old = sq(&ctx.image),
            old_plain = ctx.image,
            pat = sed_pattern(&ctx.image),
            rep = sed_replacement(new_image),
            new = new_image,
        ),
    ))
}

/// Backup, move the old data aside, new image, restore: the steps for a database whose new major cannot read the old
/// data. Without a compose file this agent can read, the change of image and the start are left to the person, and the
/// template comes in two parts around them.
fn database(ctx: &Ctx, family: Family) -> Vec<Template> {
    let Some(sp) = spec(ctx, family) else { return vec![] };
    let new_image = with_tag(&ctx.image, &ctx.to_tag);
    let c = &ctx.container;
    let dump = dump_path(ctx, sp.extension);
    let env = format!("DUMP={}; ", sq(&dump));
    let mut before = vec![
        step("Check the container and the free space", format!("docker inspect -f '{{{{.Name}}}} {{{{.State.Status}}}} {{{{.Config.Image}}}}' {c}; mkdir -p {BACKUP_DIR}; df -h {BACKUP_DIR}")),
        step(&format!("Back up the data ({}) to a file", sp.name), format!("mkdir -p {BACKUP_DIR} && {env}{}", sp.dump)),
    ];
    let moves = !sp.data.is_empty();
    if moves {
        before.push(step("Stop the container", format!("docker stop {c}")));
        before.push(step(
            "Move the old data directory aside (kept, not deleted)",
            format!("{}; test ! -e \"$SRC.pre-upgrade\" || {{ echo \"$SRC.pre-upgrade already exists: rename or remove it first\"; exit 1; }}; mv \"$SRC\" \"$SRC.pre-upgrade\" && mkdir -p \"$SRC\" && echo \"old data kept in $SRC.pre-upgrade\"", find_src(ctx, sp.data)),
        ));
    }
    let mut change = vec![];
    if let Some(r) = retag_step(ctx, &new_image) {
        change.push(r);
    }
    if let Some(up) = compose_cmd(ctx, &format!("up -d --no-deps {}", ctx.service.as_deref().map(sq).unwrap_or_else(|| sq(c)))) {
        change.push(step(&format!("Start it on {new_image}"), up));
    }
    let mut after = vec![step("Wait until it answers", sp.ready.clone())];
    if let Some(restore) = &sp.restore {
        after.push(step("Restore the backup", format!("{env}{restore}")));
    }
    after.push(step("Check the result", sp.check.clone()));

    let mut rollback = vec![step("Stop the container", format!("docker stop {c}"))];
    if moves {
        rollback.push(step(
            "Put the old data directory back",
            format!("{}; test -e \"$SRC.pre-upgrade\" || {{ echo \"$SRC.pre-upgrade not found: nothing to put back\"; exit 1; }}; mv \"$SRC\" \"$SRC.failed-upgrade\" && mv \"$SRC.pre-upgrade\" \"$SRC\" && echo \"the failed data is in $SRC.failed-upgrade\"", find_src(ctx, sp.data)),
        ));
    }
    if !ctx.compose_files.is_empty() {
        let files = ctx.compose_files.iter().map(|f| sq(f)).collect::<Vec<_>>().join(" ");
        rollback.push(step("Put the old image back in the compose file", format!("for F in {files}; do [ -f \"$F.pre-upgrade\" ] && cp -p \"$F.pre-upgrade\" \"$F\" && echo \"$F restored\"; done; true")));
        if let Some(up) = compose_cmd(ctx, &format!("up -d --no-deps {}", ctx.service.as_deref().map(sq).unwrap_or_else(|| sq(c)))) {
            rollback.push(step("Start it on the old image", up));
        }
    } else {
        rollback.push(step("Start it on the old image", format!("echo 'Put {} back in the tool that manages this stack and start it from there'", ctx.image)));
    }

    let what = format!("{} {} → {}: the new image cannot open the old data directory, so the data is dumped, the old directory is moved aside and the dump is restored into a new one.", sp.name, ctx.image, new_image);
    let what = if family == Family::Postgres { format!("{what} Restoring prints \"role postgres already exists\" for the account the new cluster already has: that is normal.") } else { what };
    if change.is_empty() {
        // no compose file to drive: the person changes the image and starts the container between the two parts
        vec![
            Template { id: "db-migrate-1".into(), title: "Part 1: back up and move the old data aside".into(), description: format!("{what} This stack has no compose file the agent can read: after part 1, change the image to {new_image} in the tool that manages it and start it, then run part 2."), steps: before, rollback: rollback.clone() },
            Template { id: "db-migrate-2".into(), title: "Part 2: restore and check (after the new image is running)".into(), description: "Run once the container is running on the new image.".into(), steps: after, rollback },
        ]
    } else {
        let steps: Vec<ShellStep> = before.into_iter().chain(change).chain(after).collect();
        vec![Template { id: "db-migrate".into(), title: format!("Migrate {} to {}", sp.name, ctx.to_tag), description: what, steps, rollback }]
    }
}

/// A new tag for anything else: back up the data directories, change the image, start it, show the logs.
fn plain(ctx: &Ctx) -> Template {
    let new_image = with_tag(&ctx.image, &ctx.to_tag);
    let c = &ctx.container;
    let mut steps = vec![step(
        "Back up the data directories (can be large)",
        format!(
            "mkdir -p {BACKUP_DIR} && docker inspect -f '{{{{range .Mounts}}}}{{{{.Source}}}}{{{{\"\\n\"}}}}{{{{end}}}}' {c} | sort -u | while read -r S; do [ -n \"$S\" ] && [ -e \"$S\" ] && tar -czf \"{BACKUP_DIR}/{c}-$(basename \"$S\").tgz\" -C \"$(dirname \"$S\")\" \"$(basename \"$S\")\" && echo \"saved $S\"; done; ls -lh {BACKUP_DIR} | tail -5"
        ),
    )];
    if let Some(r) = retag_step(ctx, &new_image) {
        steps.push(r);
    }
    if let Some(up) = compose_cmd(ctx, &format!("up -d --no-deps {}", ctx.service.as_deref().map(sq).unwrap_or_else(|| sq(c)))) {
        steps.push(step(&format!("Start it on {new_image}"), up));
    } else {
        steps.push(step("Change the image and start it", format!("echo 'Change the image to {new_image} in the tool that manages this stack, then start it from there'")));
    }
    steps.push(step("Show its state and last logs", format!("sleep 5; docker inspect -f '{{{{.State.Status}}}} {{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{end}}}}' {c}; docker logs --tail 30 {c} 2>&1")));
    let mut rollback = vec![];
    if !ctx.compose_files.is_empty() {
        let files = ctx.compose_files.iter().map(|f| sq(f)).collect::<Vec<_>>().join(" ");
        rollback.push(step("Put the old image back in the compose file", format!("for F in {files}; do [ -f \"$F.pre-upgrade\" ] && cp -p \"$F.pre-upgrade\" \"$F\" && echo \"$F restored\"; done; true")));
        if let Some(up) = compose_cmd(ctx, &format!("up -d --no-deps {}", ctx.service.as_deref().map(sq).unwrap_or_else(|| sq(c)))) {
            rollback.push(step("Start it on the old image", up));
        }
    }
    Template { id: "generic".into(), title: format!("Back up and move to {}", ctx.to_tag), description: format!("{} → {new_image}: a backup of its data directories, the new image, a look at the logs.", ctx.image), steps, rollback }
}

/// Read-only lines to look at the container before deciding anything.
fn inspect(ctx: &Ctx) -> Template {
    let c = &ctx.container;
    let mut steps = vec![
        step("State and image", format!("docker inspect -f 'name={{{{.Name}}}} image={{{{.Config.Image}}}} state={{{{.State.Status}}}} started={{{{.State.StartedAt}}}}' {c}")),
        step("Mounts (what holds its data)", format!("docker inspect -f '{{{{range .Mounts}}}}{{{{.Type}}}} {{{{.Source}}}} -> {{{{.Destination}}}}{{{{\"\\n\"}}}}{{{{end}}}}' {c}")),
        step("Last logs", format!("docker logs --tail 40 {c} 2>&1")),
        step("Free disk space", format!("df -h / {BACKUP_DIR} 2>/dev/null || df -h /")),
    ];
    if let Some(cfg) = compose_cmd(ctx, "config --quiet && echo 'compose file is valid'") {
        steps.push(step("The compose file is valid", cfg));
    }
    Template { id: "inspect".into(), title: "Look first (read-only)".into(), description: "Nothing is changed: state, mounts, logs and free space.".into(), steps, rollback: vec![] }
}

/// The templates for one container, the most specific first.
pub fn templates(ctx: &Ctx) -> Vec<Template> {
    let mut out = database(ctx, family(&ctx.image));
    out.push(plain(ctx));
    out.push(inspect(ctx));
    out
}
