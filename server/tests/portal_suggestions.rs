//! The shipped list of suggested apps must be valid catalog entries (no database needed).
use croustypatch_server::api::portal::{check_recipes, ICONS};
use serde_json::Value;

#[test]
fn every_suggestion_is_a_valid_catalog_app() {
    let list: Vec<Value> = serde_json::from_str(include_str!("../assets/app-suggestions.json")).unwrap();
    assert!(list.len() >= 30);
    let mut names = std::collections::HashSet::new();
    for s in &list {
        let name = s["name"].as_str().unwrap();
        assert!(names.insert(name.to_lowercase()), "duplicate {name}");
        assert!(ICONS.contains(&s["icon"].as_str().unwrap()), "{name}: icon");
        assert!(s["description"].as_str().unwrap().len() <= 2000 && s["category"].as_str().unwrap().len() <= 40, "{name}");
        assert_eq!(check_recipes(&s["recipes"]).ok(), Some(false), "{name}: packages only, valid names");
        for pm in ["apt", "pacman"] {
            assert!(s["recipes"].get(pm).is_some(), "{name}: both Debian and Arch are covered");
        }
    }
}
