//! When and what the update notifier says. Pure functions (standard library only), so they are tested on their own.

/// A stable fingerprint of the pending updates (FNV-1a over the sorted `name=candidate` lines): a notification is
/// sent again only when the list changes, not every half hour.
pub fn fingerprint(items: &[(String, String)]) -> String {
    let mut lines: Vec<String> = items.iter().map(|(n, c)| format!("{n}={c}\n")).collect();
    lines.sort();
    let mut h: u64 = 0xcbf29ce484222325;
    for b in lines.concat().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Notify when the user may start the updates, there are some, and this list was not announced yet.
pub fn should_notify(allowed: bool, total: usize, fingerprint: &str, last: Option<&str>) -> bool {
    allowed && total > 0 && last != Some(fingerprint)
}

/// (title, body) in French or English.
pub fn message(french: bool, total: usize, security: usize) -> (String, String) {
    let plural = total > 1;
    if french {
        let title = if security > 0 { "Mises à jour de sécurité disponibles" } else { "Mises à jour disponibles" };
        let mut body = if plural { format!("{total} mises à jour peuvent être installées sur cet ordinateur.") } else { "Une mise à jour peut être installée sur cet ordinateur.".to_string() };
        if security > 0 {
            body += &format!(" Dont {security} de sécurité.");
        }
        body += " Ouvrez le portail applicatif pour les installer.";
        (title.to_string(), body)
    } else {
        let title = if security > 0 { "Security updates available" } else { "Updates available" };
        let mut body = if plural { format!("{total} updates can be installed on this computer.") } else { "An update can be installed on this computer.".to_string() };
        if security > 0 {
            body += &format!(" {security} of them security updates.");
        }
        body += " Open the app center to install them.";
        (title.to_string(), body)
    }
}

/// French when the session language says so ("fr_FR.UTF-8", "fr").
pub fn is_french(lang: &str) -> bool {
    lang.trim().to_ascii_lowercase().starts_with("fr")
}

/// Seconds to wait between two checks, from an optional override (tests and special setups), within 1 minute..1 day.
pub fn interval_secs(override_secs: Option<&str>) -> u64 {
    override_secs.and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(30 * 60).clamp(60, 24 * 3600)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn the_fingerprint_depends_on_the_content_not_the_order() {
        let a = fingerprint(&items(&[("git", "2.50"), ("vim", "9.1")]));
        assert_eq!(a, fingerprint(&items(&[("vim", "9.1"), ("git", "2.50")])));
        assert_ne!(a, fingerprint(&items(&[("git", "2.51"), ("vim", "9.1")])), "a new version is a new list");
        assert_ne!(a, fingerprint(&items(&[("git", "2.50")])));
        assert_eq!(a.len(), 16);
        assert_eq!(fingerprint(&[]), "cbf29ce484222325", "empty input hashes to the FNV offset basis");
    }

    #[test]
    fn it_speaks_once_per_list_and_only_when_the_user_may_act() {
        assert!(should_notify(true, 3, "abc", None));
        assert!(should_notify(true, 3, "abc", Some("old")));
        assert!(!should_notify(true, 3, "abc", Some("abc")), "already announced");
        assert!(!should_notify(false, 3, "abc", None), "the administrator manages this machine: nothing to tell the user");
        assert!(!should_notify(true, 0, "abc", None), "nothing pending");
    }

    #[test]
    fn the_message_is_plain_and_mentions_security() {
        let (t, b) = message(true, 5, 2);
        assert_eq!(t, "Mises à jour de sécurité disponibles");
        assert!(b.contains("5 mises à jour") && b.contains("2 de sécurité") && b.contains("portail applicatif"), "{b}");
        let (t, b) = message(false, 1, 0);
        assert_eq!(t, "Updates available");
        assert!(b.starts_with("An update can be installed") && !b.contains("security"), "{b}");
        assert!(is_french("fr_FR.UTF-8") && is_french("FR") && !is_french("en_US.UTF-8") && !is_french(""));
    }

    #[test]
    fn the_check_interval_is_bounded() {
        assert_eq!(interval_secs(None), 1800);
        assert_eq!(interval_secs(Some("120")), 120);
        assert_eq!(interval_secs(Some("1")), 60, "never hammer the agent");
        assert_eq!(interval_secs(Some("999999")), 86400);
        assert_eq!(interval_secs(Some("soon")), 1800);
    }
}
