//! The generated PostgreSQL migration against a real Docker daemon: data is dumped, the old directory moved aside, the
//! image changed in the compose file, the new major started and the dump restored; then the rollback puts it all back.
//! Each step runs the way the agent runs it (`sh -c`, one after the other, stop at the first failure).
//!
//! Needs docker, `docker compose` and internet (postgres:16-alpine and 17-alpine). Run on a machine you can start
//! throw-away containers on, as a user that can use docker:
//!   cargo test -p croustypatch-server --test docker_migration_live -- --ignored --nocapture --test-threads=1
use croustypatch_server::docker_migration::{templates, Ctx, BACKUP_DIR};
use std::process::Command;

fn sh(cmd: &str) -> (bool, String) {
    let o = Command::new("sh").args(["-c", cmd]).output().expect("sh");
    (o.status.success(), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string())
}

struct Stack {
    dir: tempfile::TempDir,
    backups: tempfile::TempDir,
}

impl Drop for Stack {
    fn drop(&mut self) {
        let d = self.dir.path().display();
        let _ = sh(&format!("docker compose --project-directory {d} -f {d}/compose.yaml -p cptest-mig down -v --remove-orphans; docker rm -f cptest-mig; docker run --rm -v {d}:/d alpine sh -c 'rm -rf /d/data /d/data.*'"));
    }
}

#[test]
#[ignore = "needs docker, compose and internet"]
fn a_postgres_major_is_migrated_with_its_data_and_rolled_back() {
    let st = Stack { dir: tempfile::tempdir().unwrap(), backups: tempfile::tempdir().unwrap() };
    let d = st.dir.path().display().to_string();
    let file = format!("{d}/compose.yaml");
    std::fs::write(&file, "# a stack like the others\nservices:\n  db:\n    container_name: cptest-mig\n    image: postgres:16-alpine\n    environment:\n      POSTGRES_PASSWORD: secret\n    volumes:\n      - ./data:/var/lib/postgresql/data\n").unwrap();
    let compose = |tail: &str| sh(&format!("docker compose --project-directory {d} -f {file} -p cptest-mig {tail}"));
    let (ok, out) = compose("up -d --quiet-pull");
    assert!(ok, "{out}");
    let psql = |q: &str| sh(&format!("docker exec cptest-mig psql -U postgres -tA -c \"{q}\""));
    for _ in 0..30 {
        if sh("docker exec cptest-mig pg_isready -q").0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    assert!(psql("create table t(x int); insert into t values (1),(2),(3)").0);
    assert_eq!(psql("select count(*) from t").1, "3");
    let image = || sh("docker inspect -f '{{.Config.Image}}' cptest-mig").1;
    assert_eq!(image(), "postgres:16-alpine");

    let ctx = Ctx { container: "cptest-mig".into(), image: "postgres:16-alpine".into(), to_tag: "17-alpine".into(), project: Some("cptest-mig".into()), service: Some("db".into()), compose_dir: Some(d.clone()), compose_files: vec![file.clone()] };
    let t = templates(&ctx).remove(0);
    assert_eq!(t.id, "db-migrate");
    let run = |steps: &[croustypatch_common::protocol::ShellStep]| {
        for s in steps {
            // the backup directory is /var/backups/croustypatch on a host; a temporary one here (no root needed)
            let cmd = s.command.replace(BACKUP_DIR, &st.backups.path().display().to_string());
            let (ok, out) = sh(&cmd);
            println!("── {}\n{out}", s.title);
            assert!(ok, "step '{}' failed:\n{cmd}\n{out}", s.title);
        }
    };
    run(&t.steps);
    assert_eq!(image(), "postgres:17-alpine", "the container runs the new major");
    assert_eq!(psql("select count(*) from t").1, "3", "and has the data, restored from the dump");
    assert!(psql("show server_version").1.starts_with("17"));
    assert!(std::fs::read_to_string(&file).unwrap().contains("image: postgres:17-alpine") && std::path::Path::new(&format!("{file}.pre-upgrade")).is_file(), "the compose file was changed and a copy kept");
    assert!(std::path::Path::new(&format!("{d}/data.pre-upgrade")).is_dir(), "the old data directory is kept");
    assert!(std::fs::read_dir(st.backups.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with("-pre-upgrade.sql")), "the dump is in the backup directory");

    run(&t.rollback);
    assert_eq!(image(), "postgres:16-alpine", "rolled back to the old image");
    for _ in 0..30 {
        if sh("docker exec cptest-mig pg_isready -q").0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    assert_eq!(psql("select count(*) from t").1, "3", "on the old data");
    assert!(std::fs::read_to_string(&file).unwrap().contains("image: postgres:16-alpine"));
}
