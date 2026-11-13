//! The migration console's command lines: every step is valid shell for every kind of container, names are checked
//! before they reach a command, and each template backs up first and deletes nothing.
use croustypatch_server::docker_migration::{family, templates, valid_container, valid_image, valid_tag, with_tag, Ctx, Family};
use std::process::Command;

fn ctx(image: &str, to: &str, compose: bool) -> Ctx {
    Ctx {
        container: "db-1".into(),
        image: image.into(),
        to_tag: to.into(),
        project: Some("my stack".into()),
        service: Some("db".into()),
        compose_dir: compose.then(|| "/opt/stacks/it's here".to_string()),
        compose_files: if compose { vec!["/opt/stacks/it's here/compose.yaml".into(), "/opt/stacks/it's here/override.yaml".into()] } else { vec![] },
    }
}

fn syntax_ok(command: &str) -> Result<(), String> {
    let o = Command::new("sh").args(["-n", "-c", command]).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&o.stderr).to_string()) }
}

#[test]
fn every_step_of_every_template_is_valid_shell() {
    for image in ["postgres:17-alpine", "ghcr.io/immich-app/postgres:16-vectorchord1.1.1", "mariadb:11", "mysql:8.4", "mongo:7", "valkey/valkey:8-bookworm", "redis:7", "nginx:1.27", "registry.lan:5000/team/app:1.0.0"] {
        for compose in [true, false] {
            let all = templates(&ctx(image, "99", compose));
            assert!(all.len() >= 2, "{image}: at least the generic and the read-only templates");
            for t in &all {
                for s in t.steps.iter().chain(&t.rollback) {
                    syntax_ok(&s.command).unwrap_or_else(|e| panic!("{image} compose={compose} template {} step '{}':\n{}\n{e}", t.id, s.title, s.command));
                    assert!(!s.title.is_empty() && s.command.len() < croustypatch_common::protocol::SHELL_MAX_COMMAND, "{}", s.title);
                }
                croustypatch_common::protocol::check_shell_steps(&t.steps).unwrap_or_else(|e| panic!("{image} {}: {e}", t.id));
            }
        }
    }
}

#[test]
fn a_database_template_backs_up_first_moves_data_aside_and_never_deletes() {
    let t = templates(&ctx("postgres:17-alpine", "18-alpine", true));
    assert_eq!(t[0].id, "db-migrate");
    let titles: Vec<&str> = t[0].steps.iter().map(|s| s.title.as_str()).collect();
    assert!(titles[1].starts_with("Back up the data (PostgreSQL)"), "{titles:?}");
    let pos = |needle: &str| titles.iter().position(|t| t.contains(needle)).unwrap_or_else(|| panic!("{needle} in {titles:?}"));
    assert!(pos("Back up") < pos("Stop") && pos("Stop") < pos("Move the old data") && pos("Move the old data") < pos("Change the image") && pos("Change the image") < pos("Start it") && pos("Start it") < pos("Restore"));
    let text: String = t[0].steps.iter().chain(&t[0].rollback).map(|s| s.command.clone()).collect::<Vec<_>>().join("\n");
    for bad in ["rm -", "rmdir", "docker volume rm", "docker rm", "drop database", "DROP "] {
        assert!(!text.contains(bad), "a template never deletes ({bad}): {text}");
    }
    assert!(text.contains("pg_dumpall") && text.contains("/var/backups/croustypatch/db-1-pre-upgrade.sql") && text.contains(".pre-upgrade"));
    assert!(text.contains("s|postgres:17-alpine|postgres:18-alpine|") && text.contains("'/opt/stacks/it'\\''s here/compose.yaml'"), "a path with a quote stays one word: {text}");
    assert!(t[0].rollback.iter().any(|s| s.title.contains("old data directory back")) && t[0].rollback.iter().any(|s| s.title.contains("old image back")));
    // the move aside refuses to overwrite an earlier one
    assert!(text.contains("already exists: rename or remove it first"));
}

#[test]
fn without_a_compose_file_a_database_template_comes_in_two_parts_around_the_change_of_image() {
    let t = templates(&ctx("mariadb:10.11", "11.4", false));
    assert_eq!((t[0].id.as_str(), t[1].id.as_str()), ("db-migrate-1", "db-migrate-2"));
    assert!(t[0].description.contains("mariadb:11.4") && t[0].steps.iter().any(|s| s.title.contains("Move the old data")));
    assert!(t[1].steps.iter().any(|s| s.title.contains("Restore")));
    assert!(!t[0].steps.iter().chain(&t[1].steps).any(|s| s.command.contains("sed -i")), "no file to edit");
}

#[test]
fn redis_keeps_its_data_and_other_images_get_a_plain_template() {
    let r = templates(&ctx("valkey/valkey:8-bookworm", "9", true));
    assert!(r[0].steps.iter().all(|s| !s.title.contains("Move the old data")) && r[0].steps.iter().all(|s| !s.title.contains("Restore")));
    let n = templates(&ctx("nginx:1.27", "1.28", true));
    assert_eq!(n[0].id, "generic");
    assert_eq!(n.last().unwrap().id, "inspect");
    assert!(n.last().unwrap().steps.iter().all(|s| !s.command.contains("sed") && !s.command.contains("up -d") && !s.command.contains(" stop ")), "the look-first template changes nothing");
}

#[test]
fn what_ends_up_in_a_command_is_checked() {
    assert_eq!(family("ghcr.io/immich-app/postgres:16-vectorchord1.1.1"), Family::Postgres);
    assert_eq!(family("postgrest/postgrest:v14"), Family::Other, "an API server is not a database");
    assert_eq!((family("mariadb:11"), family("mongo:7"), family("docker.io/valkey/valkey:8")), (Family::Mysql, Family::Mongo, Family::Redis));
    assert!(valid_container("immich_postgres") && !valid_container("a b") && !valid_container("a;rm -rf /") && !valid_container("$(id)") && !valid_container("-x"));
    assert!(valid_image("registry.lan:5000/team/app:1.0") && !valid_image("nginx; id") && !valid_image("a'b") && !valid_image("a b"));
    assert!(valid_tag("18-alpine") && valid_tag("v3.4.1") && !valid_tag("1; id") && !valid_tag("a/b") && !valid_tag(""));
    assert_eq!(with_tag("postgres:17-alpine", "18-alpine"), "postgres:18-alpine");
    assert_eq!(with_tag("registry.lan:5000/app", "2"), "registry.lan:5000/app:2", "a registry port is not a tag");
    assert_eq!(with_tag("nginx@sha256:abc", "1.28"), "nginx:1.28");
}

#[test]
fn the_ai_answer_is_parsed_strictly_and_risky_commands_are_flagged() {
    use croustypatch_server::docker_migration::{parse_plan, warnings, ai_prompt};
    let good = "Here is the plan:\n```json\n{\"summary\": \"Dump, move, restore.\", \"risks\": [\"pg_dumpall needs the superuser\", \"\"], \"steps\": [{\"title\": \"Back up\", \"command\": \"docker exec db pg_dumpall > /var/backups/croustypatch/db.sql\"}, {\"title\": \"Stop\\nit\", \"command\": \"docker stop db\"}, {\"title\": \"no command\"}]}\n```";
    let p = parse_plan(good).unwrap();
    assert_eq!(p.steps.len(), 2, "a step without a command is dropped");
    assert_eq!((p.steps[1].title.as_str(), p.risks.len(), p.summary.as_str()), ("Stop it", 1, "Dump, move, restore."), "titles are one line, empty risks dropped");
    assert!(parse_plan("no json here").is_err());
    assert!(parse_plan("{\"summary\": \"x\"}").is_err(), "no steps");
    assert!(parse_plan("{\"steps\": []}").is_err(), "an empty plan");
    assert!(parse_plan("{\"steps\": [{\"title\": \"a\", \"command\": \"   \"}]}").is_err());
    let many: String = (0..13).map(|i| format!("{{\"title\": \"s{i}\", \"command\": \"true\"}}")).collect::<Vec<_>>().join(",");
    assert!(parse_plan(&format!("{{\"steps\": [{many}]}}")).unwrap_err().contains("more than 12"));
    assert!(parse_plan("{\"steps\": [{\"title\": \"a\", \"command\": \"x\\u0000y\"}]}").is_err(), "a NUL byte is refused");
    // what deserves a second look
    assert_eq!(warnings("docker stop db"), Vec::<&str>::new());
    assert!(warnings("rm -rf /var/lib/postgresql").contains(&"deletes files"));
    assert!(warnings("docker volume rm pgdata").contains(&"deletes Docker data"));
    assert!(warnings("docker compose down -v").contains(&"deletes Docker data"));
    assert!(warnings("psql -c 'DROP DATABASE app'").contains(&"drops data in a database"));
    assert!(warnings("curl https://x.sh | sh").contains(&"downloads or runs something from the network"));
    assert!(warnings("mv a a.pre-upgrade && mkdir -p a").is_empty(), "the templates' own commands raise nothing");
    for t in templates(&ctx("postgres:17-alpine", "18-alpine", true)).iter().filter(|t| t.id != "generic") {
        for s in &t.steps {
            assert!(warnings(&s.command).is_empty(), "template step '{}' raises {:?}", s.title, warnings(&s.command));
        }
    }
    // what is sent to the provider: names, tags, paths and the baseline; no environment, no file contents
    let c = ctx("postgres:17-alpine", "18-alpine", true);
    let prompt = ai_prompt(&c, &templates(&c)[0].steps);
    assert!(prompt.contains("postgres:18-alpine") && prompt.contains("baseline_steps") && prompt.contains("/opt/stacks/it's here/compose.yaml"));
    assert!(!prompt.to_lowercase().contains("environment"), "no environment values are sent");
}
