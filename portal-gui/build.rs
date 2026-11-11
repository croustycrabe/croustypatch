fn main() {
    // Declaring the app's commands makes Tauri require an explicit permission for each one: the window can call
    // these three and nothing else (see capabilities/main.json).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["list", "install", "uninstall", "updates", "update", "check"]))).expect("tauri build");
}
