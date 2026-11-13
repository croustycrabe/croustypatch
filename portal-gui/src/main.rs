//! CroustyPatch desktop app center, a native GPUI window.
//!
//! Exactly as powerful as the terminal app center: it talks to the agent's local socket
//! (`/run/croustypatch/portal.sock`), and the agent identifies the user from the socket's peer credentials, never
//! from anything this program says. The window can ask for six operations (list, install, uninstall, updates, update,
//! check: see `backend::Op`); no shell, file system or network access is exposed to it.
mod assets;
mod backend;
mod i18n;
mod model;
mod notifier;
mod notify_logic;
mod settings;
mod theme;
mod ui;

pub(crate) use backend::ask;

fn main() {
    // Started at login by the desktop autostart entry: no window, only notifications.
    if std::env::args().any(|a| a == "--notifier") {
        notifier::run();
        return;
    }
    gpui::Application::new().with_assets(assets::Assets).run(|cx| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        ui::open(cx);
    });
}
