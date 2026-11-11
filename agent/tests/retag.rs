//! Applying a suggested Docker version: the compose file change, planned and written with care.
use croustypatch_agent::modules::retag::{check_refs, plan_all, plan_one, revert, split_tag, valid_tag, write_file, FileEdit, Outcome};

const IMMICH: &str = "name: immich
services:
  immich-server:
    container_name: immich_server
    image: ghcr.io/immich-app/immich-server:v3.3.1
    volumes:
      - ./library:/data
    environment:
      TZ: Europe/Paris
    ports:
      - \"2283:2283\"
    depends_on:
      - redis
      - database
  immich-machine-learning:
    container_name: immich_machine_learning
    image: ghcr.io/immich-app/immich-machine-learning:v3.3.1 # same as the server
    volumes:
      - model-cache:/cache
  redis:
    image: docker.io/valkey/valkey:8-bookworm
  database:
    image: ghcr.io/immich-app/postgres:16-vectorchord1.1.1
volumes:
  model-cache:
";

const SERVER_OLD: &str = "ghcr.io/immich-app/immich-server:v3.3.1";
const SERVER_NEW: &str = "ghcr.io/immich-app/immich-server:v3.4.1";

fn one(file: &str, service: &str, old: &str, new: &str) -> Result<(Outcome, String), String> {
    let mut files = vec![("/stack/compose.yaml".to_string(), file.to_string())];
    plan_one(service, old, new, &mut files).map(|o| (o, files.remove(0).1))
}

/// The lines that differ between two texts.
fn changed_lines(a: &str, b: &str) -> Vec<(String, String)> {
    assert_eq!(a.lines().count(), b.lines().count(), "no line is added or removed");
    a.lines().zip(b.lines()).filter(|(x, y)| x != y).map(|(x, y)| (x.to_string(), y.to_string())).collect()
}

#[test]
fn references_are_split_and_checked() {
    assert_eq!(split_tag("ghcr.io/x/y:v3.3.1"), Some(("ghcr.io/x/y", "v3.3.1")));
    assert_eq!(split_tag("nginx:1.27"), Some(("nginx", "1.27")));
    assert_eq!(split_tag("localhost:5000/app"), None, "a registry port is not a tag");
    assert_eq!(split_tag("localhost:5000/app:2.0"), Some(("localhost:5000/app", "2.0")));
    assert_eq!(split_tag("nginx"), None);
    assert!(valid_tag("v3.4.1") && valid_tag("17-alpine") && valid_tag("2026.10.12-sha-3699278") && !valid_tag("") && !valid_tag("-x") && !valid_tag("a b") && !valid_tag("a/b") && !valid_tag(&"a".repeat(129)));
    assert!(check_refs(SERVER_OLD, SERVER_NEW).is_ok());
    assert!(check_refs(SERVER_OLD, "ghcr.io/immich-app/immich-machine-learning:v3.4.1").unwrap_err().contains("not the same image"), "only the tag may change");
    assert!(check_refs(SERVER_OLD, "docker.io/immich-app/immich-server:v3.4.1").is_err(), "nor the registry");
    assert!(check_refs(SERVER_OLD, SERVER_OLD).unwrap_err().contains("runs already"));
    assert!(check_refs(SERVER_OLD, "ghcr.io/immich-app/immich-server@sha256:abcd").is_err(), "a digest is not a tag");
    assert!(check_refs(SERVER_OLD, "ghcr.io/immich-app/immich-server:v3.4.1; rm -rf /").is_err());
    assert!(check_refs(SERVER_OLD, "ghcr.io/immich-app/immich-server").is_err());
    assert!(check_refs("nginx", "nginx:1.28").is_err(), "the running reference needs a tag too");
}

#[test]
fn only_the_tag_of_the_one_service_changes() {
    let (outcome, edited) = one(IMMICH, "immich-server", SERVER_OLD, SERVER_NEW).unwrap();
    assert_eq!(outcome, Outcome::Changed { path: "/stack/compose.yaml".into() });
    assert_eq!(changed_lines(IMMICH, &edited), vec![(format!("    image: {SERVER_OLD}"), format!("    image: {SERVER_NEW}"))], "one line, nothing else: not the sibling service on the same version, not a comment");
    assert!(edited.ends_with("volumes:\n  model-cache:\n"), "the end of the file is untouched");
    // the sibling is a service of its own, with its own trailing comment kept
    let (_, ml) = one(IMMICH, "immich-machine-learning", "ghcr.io/immich-app/immich-machine-learning:v3.3.1", "ghcr.io/immich-app/immich-machine-learning:v3.4.1").unwrap();
    assert_eq!(changed_lines(IMMICH, &ml), vec![("    image: ghcr.io/immich-app/immich-machine-learning:v3.3.1 # same as the server".into(), "    image: ghcr.io/immich-app/immich-machine-learning:v3.4.1 # same as the server".into())]);
    // a database: the same mechanism (the suggestion layer decides whether a major should be offered)
    let (_, db) = one(IMMICH, "database", "ghcr.io/immich-app/postgres:16-vectorchord1.1.1", "ghcr.io/immich-app/postgres:16-vectorchord1.2.0").unwrap();
    assert_eq!(changed_lines(IMMICH, &db).len(), 1);
}

#[test]
fn quotes_comments_indentation_and_line_endings_are_kept() {
    for (line, expect) in [
        ("    image: \"web:1.0\"", "    image: \"web:2.0\""),
        ("    image: 'web:1.0'", "    image: 'web:2.0'"),
        ("    image:   web:1.0   # pinned on purpose", "    image:   web:2.0   # pinned on purpose"),
        ("    image: \"web:1.0\" # q", "    image: \"web:2.0\" # q"),
        ("    image:\tweb:1.0", "    image:\tweb:2.0"),
    ] {
        let file = format!("services:\n  web:\n{line}\n    ports: []\n");
        let (_, out) = one(&file, "web", "web:1.0", "web:2.0").unwrap_or_else(|e| panic!("{line}: {e}"));
        assert_eq!(out, format!("services:\n  web:\n{expect}\n    ports: []\n"), "{line}");
    }
    // Windows line endings and a file that does not end with a newline
    let crlf = "services:\r\n  web:\r\n    image: web:1.0\r\n    ports: []";
    let (_, out) = one(crlf, "web", "web:1.0", "web:2.0").unwrap();
    assert_eq!(out, "services:\r\n  web:\r\n    image: web:2.0\r\n    ports: []");
    // four-space indentation, comments and blank lines between the keys
    let wide = "# stack\nservices:\n\n    # the web part\n    web:\n        # which image\n        image: web:1.0\n\n        restart: always\n";
    let (_, out) = one(wide, "web", "web:1.0", "web:2.0").unwrap();
    assert_eq!(changed_lines(wide, &out), vec![("        image: web:1.0".into(), "        image: web:2.0".into())]);
}

#[test]
fn a_file_that_already_says_the_new_version_only_needs_the_container_recreated() {
    let file = "services:\n  web:\n    image: web:2.0\n";
    let mut files = vec![("/s/compose.yaml".to_string(), file.to_string())];
    assert_eq!(plan_one("web", "web:1.0", "web:2.0", &mut files).unwrap(), Outcome::AlreadyThere { path: "/s/compose.yaml".into() });
    assert_eq!(files[0].1, file, "nothing written");
}

#[test]
fn everything_that_is_not_a_plain_image_line_is_refused_with_the_reason() {
    let svc = |body: &str| format!("services:\n  web:\n{body}\n");
    for (name, file, why) in [
        ("a variable for the tag", svc("    image: web:${WEB_VERSION:-1.0}"), "variable"),
        ("a variable for everything", svc("    image: ${WEB_IMAGE}"), "variable"),
        ("another version in the file than the container runs", svc("    image: web:0.9"), "not created from this line"),
        ("an image from somewhere else", svc("    image: other/web:1.0"), "not created from this line"),
        ("an alias", "x-img: &img web:1.0\nservices:\n  web:\n    image: *img\n".to_string(), "anchor"),
        ("a merge key and no image", "x-base: &base\n  image: web:1.0\nservices:\n  web:\n    <<: *base\n    ports: []\n".to_string(), "merge key"),
        ("extends", svc("    extends:\n      service: base\n    ports: []"), "extends"),
        ("a service that is built", svc("    build: .\n    ports: []"), "no image"),
        ("the image on the next line", svc("    image:\n      web:1.0"), "next line"),
        ("an escape in a quoted value", svc("    image: \"web:1.0\\n\""), "plain string"),
        ("a service written inline", "services:\n  web: {image: \"web:1.0\"}\n".to_string(), "inline"),
        ("services written inline", "services: {web: {image: \"web:1.0\"}}\n".to_string(), "inline"),
        ("tabs", "services:\n\tweb:\n\t\timage: web:1.0\n".to_string(), "tabs"),
    ] {
        let e = one(&file, "web", "web:1.0", "web:2.0").expect_err(name);
        assert!(e.contains(why), "{name}: {e}");
    }
    // not there at all
    assert!(one(&svc("    image: web:1.0"), "worker", "web:1.0", "web:2.0").unwrap_err().contains("no `image:` line for service worker"));
    assert!(one("version: '3'\nnetworks: {}\n", "web", "web:1.0", "web:2.0").unwrap_err().contains("no `image:` line"));
    // a key that merely starts with image, or an image nested deeper, is not the image
    assert!(one(&svc("    image_pull_policy: always\n    build:\n      context: .\n      image: web:1.0"), "web", "web:1.0", "web:2.0").unwrap_err().contains("no image"), "only a direct child counts");
}

#[test]
fn several_files_are_read_in_order_and_the_last_definition_wins() {
    let base = "services:\n  web:\n    image: web:1.0\n  db:\n    image: db:5\n";
    let over = "services:\n  web:\n    image: web:1.1\n";
    let mut files = vec![("/s/compose.yaml".to_string(), base.to_string()), ("/s/compose.override.yaml".to_string(), over.to_string())];
    // the container was created from the override: that is the line to change, and the base stays as it is
    assert_eq!(plan_one("web", "web:1.1", "web:1.2", &mut files).unwrap(), Outcome::Changed { path: "/s/compose.override.yaml".into() });
    assert_eq!(files[0].1, base);
    assert_eq!(files[1].1, "services:\n  web:\n    image: web:1.2\n");
    // running what the base says while the override says otherwise: not created from the line in force
    let mut files = vec![("/s/compose.yaml".to_string(), base.to_string()), ("/s/compose.override.yaml".to_string(), over.to_string())];
    assert!(plan_one("web", "web:1.0", "web:1.2", &mut files).unwrap_err().contains("not created from this line"));
    // a service only in the base is changed in the base, through an override that does not mention it
    let mut files = vec![("/s/compose.yaml".to_string(), base.to_string()), ("/s/compose.override.yaml".to_string(), "services:\n  web:\n    environment:\n      A: b\n".to_string())];
    assert_eq!(plan_one("db", "db:5", "db:6", &mut files).unwrap(), Outcome::Changed { path: "/s/compose.yaml".into() });
}

#[test]
fn containers_of_one_file_are_planned_one_after_the_other_and_the_file_is_edited_once() {
    let read = |paths: &[String]| -> Result<Vec<(String, String)>, String> { Ok(paths.iter().map(|p| (p.clone(), IMMICH.to_string())).collect()) };
    let paths = vec!["/opt/stacks/immich/compose.yaml".to_string()];
    let both = vec![
        ("immich-server".to_string(), SERVER_OLD.to_string(), SERVER_NEW.to_string(), paths.clone()),
        ("immich-machine-learning".to_string(), "ghcr.io/immich-app/immich-machine-learning:v3.3.1".to_string(), "ghcr.io/immich-app/immich-machine-learning:v3.4.1".to_string(), paths.clone()),
    ];
    let (edits, outcomes) = plan_all(&both, &read).unwrap();
    assert_eq!(outcomes.len(), 2);
    assert_eq!(edits.len(), 1, "one file, one edit");
    assert_eq!(edits[0].original, IMMICH, "the original is the text on disk");
    assert_eq!(changed_lines(IMMICH, &edits[0].edited).len(), 2, "both services moved together");
    assert!(edits[0].edited.contains(SERVER_NEW) && edits[0].edited.contains("immich-machine-learning:v3.4.1 # same as the server"));
    // all or nothing: the second cannot be changed, so nothing is planned for the first either
    let bad = vec![both[0].clone(), ("immich-machine-learning".to_string(), "ghcr.io/immich-app/immich-machine-learning:v9".to_string(), "ghcr.io/immich-app/immich-machine-learning:v10".to_string(), paths)];
    let e = plan_all(&bad, &read).unwrap_err();
    assert!(e.starts_with("immich-machine-learning:") && e.contains("not created from this line"), "{e}");
    // nothing to change at all (the file already says it): no edit, the container is still recreated
    let there = |paths: &[String]| -> Result<Vec<(String, String)>, String> { Ok(paths.iter().map(|p| (p.clone(), IMMICH.replace("immich-server:v3.3.1", "immich-server:v3.4.1"))).collect()) };
    let (edits, outcomes) = plan_all(&both[..1], &there).unwrap();
    assert!(edits.is_empty() && matches!(outcomes[0], Outcome::AlreadyThere { .. }));
    // a file that cannot be read stops the plan
    let missing = |_: &[String]| -> Result<Vec<(String, String)>, String> { Err("cannot read /nope: No such file or directory".into()) };
    assert!(plan_all(&both[..1], &missing).unwrap_err().contains("cannot read"));
}

#[test]
fn a_file_is_written_whole_with_its_mode_and_put_back_only_if_nobody_touched_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compose.yaml").to_string_lossy().to_string();
    std::fs::write(&path, IMMICH).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut files = vec![(path.clone(), IMMICH.to_string())];
    plan_one("immich-server", SERVER_OLD, SERVER_NEW, &mut files).unwrap();
    let edit = FileEdit { path: path.clone(), original: IMMICH.to_string(), edited: files.remove(0).1 };
    write_file(&path, &edit.edited).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edit.edited);
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640, "the mode is kept");
    let leftovers: Vec<String> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).filter(|n| n != "compose.yaml").collect();
    assert!(leftovers.is_empty(), "no temporary file is left behind: {leftovers:?}");

    // the stored form is what the restore point keeps: the original, and a hash of what was written
    let stored = edit.stored();
    assert_eq!(stored.original, IMMICH);
    revert(&stored).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), IMMICH, "put back");
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o640);
    revert(&stored).unwrap(); // already the original: nothing to do, no error
    // somebody edited the file after the update: it is not overwritten
    write_file(&path, &edit.edited).unwrap();
    std::fs::write(&path, format!("{}# edited by hand\n", edit.edited)).unwrap();
    let e = revert(&stored).unwrap_err().to_string();
    assert!(e.contains("was changed after the update") && e.contains("by hand"), "{e}");
    assert!(std::fs::read_to_string(&path).unwrap().ends_with("# edited by hand\n"), "left as it is");
    // a file that is gone cannot be put back, and says so
    std::fs::remove_file(&path).unwrap();
    assert!(revert(&stored).is_err());
}

#[test]
fn the_edit_survives_the_shape_of_a_real_multi_service_stack() {
    // a stack like supabase's: many services, an unrelated image: key in an environment block, comments everywhere
    let stack = "name: supabase
services:
  studio:
    container_name: supabase-studio
    image: supabase/studio:2026.10.12-sha-3699278
    healthcheck:
      test: [\"CMD\", \"node\", \"-e\", \"fetch('http://localhost:3000/api/platform/profile')\"]
    environment:
      STUDIO_PG_META_URL: http://meta:8080
      image: not-an-image-line-either
  auth:
    container_name: supabase-auth
    image: supabase/gotrue:v2.197.0
    depends_on:
      db:
        condition: service_healthy
  rest:
    image: postgrest/postgrest:v14.18 # PostgREST
    command: [\"postgrest\"]
  db:
    # the database
    image: supabase/postgres:17.6.1.136
    volumes:
      - ./volumes/db/data:/var/lib/postgresql/data:Z
";
    let (_, out) = one(stack, "auth", "supabase/gotrue:v2.197.0", "supabase/gotrue:v2.200.1").unwrap();
    assert_eq!(changed_lines(stack, &out), vec![("    image: supabase/gotrue:v2.197.0".into(), "    image: supabase/gotrue:v2.200.1".into())]);
    let (_, out) = one(stack, "db", "supabase/postgres:17.6.1.136", "supabase/postgres:17.11.0.019").unwrap();
    assert_eq!(changed_lines(stack, &out).len(), 1);
    let (_, out) = one(stack, "rest", "postgrest/postgrest:v14.18", "postgrest/postgrest:v14.19").unwrap();
    assert!(out.contains("postgrest/postgrest:v14.19 # PostgREST"));
    // the environment entry called image is not the service's image
    let (_, out) = one(stack, "studio", "supabase/studio:2026.10.12-sha-3699278", "supabase/studio:2026.11.1").unwrap();
    assert!(out.contains("supabase/studio:2026.11.1") && out.contains("image: not-an-image-line-either"));
}
