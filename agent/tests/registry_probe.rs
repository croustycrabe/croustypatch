//! Probe of the registry logic against real registries (needs internet):
//!   CP_IMAGES="ghcr.io/immich-app/immich-server:v3.4.1 postgres:17-alpine" cargo test -p croustypatch-agent --test registry_probe -- --ignored --nocapture
use croustypatch_agent::modules::registry;

#[tokio::test]
#[ignore = "needs internet and CP_IMAGES"]
async fn what_the_registry_says_for_each_image() {
    for r in std::env::var("CP_IMAGES").unwrap_or_default().split_whitespace() {
        let img = registry::parse(r).unwrap();
        let digest = registry::remote_digest(&img).await.map(|d| d[..19].to_string()).unwrap_or_else(|e| format!("ERR {e:#}"));
        println!("{r}\n    lookup: {:?}", croustypatch_agent::modules::docker::newer_version_checked(&img).await);
        match registry::list_tags(&img).await {
            Ok(tags) => {
                let mut shaped: Vec<(Vec<u64>, &String)> = tags.iter().filter_map(|t| registry::tag_shape(t).map(|(_, n)| (n, t))).collect();
                shaped.sort();
                let top: Vec<&str> = shaped.iter().rev().take(4).map(|(_, t)| t.as_str()).collect();
                println!("{r}\n    digest {digest} · {} same-shape tags · top {top:?} · newer {:?}", tags.len(), registry::newer_version(&img.tag, &tags));
            }
            Err(e) => println!("{r}\n    digest {digest} · TAGS ERR {e:#}"),
        }
    }
}
