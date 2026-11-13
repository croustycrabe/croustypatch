//! The login entry of the update notifier, added by the agent's self-update on laptops enrolled before 0.7.
use croustypatch_agent::selfupdate::ensure_notifier_entry;

#[test]
fn the_notifier_entry_is_added_once_and_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("etc/xdg/autostart/croustypatch-update-notifier.desktop");
    assert!(ensure_notifier_entry(&entry).unwrap(), "written, with its directory");
    let text = std::fs::read_to_string(&entry).unwrap();
    assert!(text.contains("Exec=/usr/local/bin/croustypatch-portal-gui --notifier") && text.contains("NoDisplay=true"), "{text}");
    assert!(text.starts_with("[Desktop Entry]\n") && text.contains("Type=Application"));
    // an administrator's own version stays
    std::fs::write(&entry, "[Desktop Entry]\nHidden=true\n").unwrap();
    assert!(!ensure_notifier_entry(&entry).unwrap());
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), "[Desktop Entry]\nHidden=true\n");
}

#[test]
fn the_entry_matches_the_one_the_installer_writes() {
    let script = include_str!("../../server/assets/install-agent.sh");
    let installer: String = script.split("cat > /etc/xdg/autostart/croustypatch-update-notifier.desktop <<'AUTOSTART'\n").nth(1).unwrap().split("\nAUTOSTART").next().unwrap().to_string() + "\n";
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("n.desktop");
    ensure_notifier_entry(&entry).unwrap();
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), installer, "keep selfupdate.rs and install-agent.sh in step");
}
