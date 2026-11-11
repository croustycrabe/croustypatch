//! Against the real registries (needs internet): `cargo test -p croustypatch-agent --test registry_live -- --ignored --nocapture`.
use croustypatch_agent::modules::registry;

#[tokio::test]
#[ignore = "needs internet"]
async fn newest_versions_of_real_images() {
    for image in [
        "ghcr.io/immich-app/immich-server:v3.3.1", "ghcr.io/immich-app/postgres:16-vectorchord1.1.1", "docker.io/valkey/valkey:8-bookworm", "cloudreve/cloudreve:4.18.0",
        "supabase/postgres:17.6.1.136", "supabase/studio:2026.10.12-sha-3699278", "postgres:17-alpine", "ghcr.io/gethomepage/homepage:v2.6.0", "dpage/pgadmin4:9.18", "crazymax/diun:4",
    ] {
        let img = registry::parse(image).unwrap();
        let t0 = std::time::Instant::now();
        match registry::list_tags(&img).await {
            Ok(tags) => println!("{image:<50} {:>5} tags in {:>5} ms -> {:?}", tags.len(), t0.elapsed().as_millis(), registry::newer_version(&img.tag, &tags)),
            Err(e) => println!("{image:<50} ERROR {e:#}"),
        }
    }
}
