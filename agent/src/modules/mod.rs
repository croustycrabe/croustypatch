//! Optional modules (spec docker-pve §1): each is detected on the host and reported in the facts; the server's
//! module switches (Settings > Modules) decide whether their data is used.
pub mod aur;
pub mod docker;
pub mod helper_scripts;
pub mod openscap;
pub mod registry;
pub mod retag;

/// Modules present on this host.
pub fn detected() -> Vec<String> {
    let mut m = vec![];
    if docker::available() {
        m.push("docker".to_string());
    }
    if openscap::available() {
        m.push("openscap".to_string());
    }
    if helper_scripts::available() {
        m.push("helper-script".to_string());
    }
    m
}
