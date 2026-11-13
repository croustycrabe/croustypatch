//! The window's texts, French and English. Keys are looked up at draw time, so a language change shows at once.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Fr,
    En,
}

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::Fr => "fr",
            Lang::En => "en",
        }
    }
    pub fn from_code(c: &str) -> Option<Lang> {
        match c {
            "fr" => Some(Lang::Fr),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
    /// French when the locale says so (LC_ALL, LC_MESSAGES, LANG), English otherwise.
    pub fn from_locale(v: &str) -> Lang {
        if v.to_lowercase().starts_with("fr") { Lang::Fr } else { Lang::En }
    }
    pub fn other(self) -> Lang {
        match self {
            Lang::Fr => Lang::En,
            Lang::En => Lang::Fr,
        }
    }
}

/// The text for `key`, with `{name}` placeholders replaced from `vars`. An unknown key shows as itself.
pub fn tr(lang: Lang, key: &str, vars: &[(&str, String)]) -> String {
    let table = if lang == Lang::Fr { FR } else { EN };
    let mut s = table.iter().find(|(k, _)| *k == key).map(|(_, v)| (*v).to_string()).unwrap_or_else(|| key.to_string());
    for (name, value) in vars {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

const FR: &[(&str, &str)] = &[
    ("title", "Portail applicatif"),
    ("host", "Machine {h} · demandes au nom de votre compte"),
    ("all", "Toutes"),
    ("search", "Rechercher une application"),
    ("refresh", "Actualiser"),
    ("present", "Présent"),
    ("partial", "Partiel {p}/{t}"),
    ("available", "Disponible"),
    ("working", "En cours…"),
    ("install", "Installer"),
    ("uninstall", "Désinstaller"),
    ("complete", "Compléter"),
    ("cancel", "Annuler"),
    ("update", "Mettre à jour"),
    ("update_avail", "Mise à jour {v}"),
    ("confirm_update", "Mettre à jour {n} ?"),
    ("confirm_install", "Installer {n} ?"),
    ("confirm_uninstall", "Désinstaller {n} ?"),
    ("confirm_complete", "Compléter {n} ?"),
    ("packages", "Paquets installés par le gestionnaire du système :"),
    ("command", "L’administrateur a prévu cette commande, exécutée par l’agent :"),
    ("check", "Vérifié ensuite : {p} doit être utilisable par votre compte."),
    ("missing", "Manquants : {m}"),
    ("status", "État et notifications"),
    ("clear", "Effacer"),
    ("idle", "Aucune opération en cours."),
    ("stage_queued", "en file d’attente"),
    ("stage_sent", "transmise à l’agent"),
    ("stage_running", "en cours"),
    ("stage_soaking", "vérification"),
    ("installing", "Installation de {n}"),
    ("removing", "Désinstallation de {n}"),
    ("ok_install", "{n} installé"),
    ("ok_uninstall", "{n} désinstallé"),
    ("failed", "Échec pour {n} : {m}"),
    ("cancelled", "{n} : demande annulée"),
    ("sent", "Demande envoyée pour {n}."),
    ("opened", "Déjà présentes sur cette machine : {l}"),
    ("none_present", "Aucune application du catalogue n’est encore présente."),
    ("empty", "Aucune application proposée pour cette machine."),
    ("no_match", "Aucune application ne correspond."),
    ("unreachable", "L’agent CroustyPatch ne répond pas sur cette machine ({e})."),
    ("command_badge", "commande"),
    ("tab_apps", "Applications"),
    ("tab_updates", "Mises à jour"),
    ("upd_title", "Mises à jour de cet ordinateur"),
    ("upd_sub", "{n} mise(s) à jour disponible(s){sec}"),
    ("upd_sec", " dont {s} de sécurité"),
    ("upd_none", "Cet ordinateur est à jour."),
    ("upd_check", "Rechercher"),
    ("upd_checking", "Recherche…"),
    ("upd_go", "Mettre à jour ({n})"),
    ("upd_checked", "Dernière recherche : {d}"),
    ("upd_managed", "Les mises à jour de cet ordinateur sont gérées par votre administrateur : vous pouvez les voir ici, mais pas les lancer."),
    ("upd_reboot", "Un redémarrage est nécessaire pour terminer les mises à jour."),
    ("upd_security", "Sécurité"),
    ("confirm_upd", "Mettre à jour {n} paquet(s) ?"),
    ("upd_body", "L’agent les installe avec un point de restauration et revient en arrière en cas de problème. L’ordinateur peut demander un redémarrage ensuite."),
    ("upd_running", "Mise à jour du système"),
    ("ok_upd", "Mises à jour installées"),
    ("fail_upd", "La mise à jour a échoué : {m}"),
    ("upd_sent", "Mise à jour demandée ({n} paquet(s))."),
    ("check_started", "Recherche de mises à jour lancée…"),
    ("check_done", "Recherche terminée : {n} mise(s) à jour disponible(s)."),
    ("upd_unavailable", "Les mises à jour ne sont pas disponibles sur cet ordinateur ({e})."),
    ("theme_system", "Thème : celui du système"),
    ("theme_light", "Thème : clair"),
    ("theme_dark", "Thème : sombre"),
    ("lang_title", "Langue : français (cliquer pour passer à l’anglais)"),
    ("filter", "Filtrer"),
    ("f_all", "Toutes"),
    ("f_installed", "Installées"),
    ("f_updates", "À mettre à jour"),
    ("f_available", "Non installées"),
    ("sort", "Trier"),
    ("sort_name", "Nom"),
    ("sort_status", "État"),
    ("updates_n", "{n} à mettre à jour"),
    ("empty_filter", "Aucune application dans ce filtre."),
];

const EN: &[(&str, &str)] = &[
    ("title", "App center"),
    ("host", "Machine {h} · requests made as your account"),
    ("all", "All"),
    ("search", "Search an application"),
    ("refresh", "Refresh"),
    ("present", "Present"),
    ("partial", "Partial {p}/{t}"),
    ("available", "Available"),
    ("working", "Working…"),
    ("install", "Install"),
    ("uninstall", "Uninstall"),
    ("complete", "Complete"),
    ("cancel", "Cancel"),
    ("update", "Update"),
    ("update_avail", "Update {v}"),
    ("confirm_update", "Update {n}?"),
    ("confirm_install", "Install {n}?"),
    ("confirm_uninstall", "Uninstall {n}?"),
    ("confirm_complete", "Complete {n}?"),
    ("packages", "Packages installed by the system’s package manager:"),
    ("command", "Your administrator set up this command, run by the agent:"),
    ("check", "Checked afterwards: {p} must be usable by your account."),
    ("missing", "Missing: {m}"),
    ("status", "Status and notifications"),
    ("clear", "Clear"),
    ("idle", "No operation in progress."),
    ("stage_queued", "queued"),
    ("stage_sent", "handed to the agent"),
    ("stage_running", "running"),
    ("stage_soaking", "checking"),
    ("installing", "Installing {n}"),
    ("removing", "Removing {n}"),
    ("ok_install", "{n} installed"),
    ("ok_uninstall", "{n} uninstalled"),
    ("failed", "{n} failed: {m}"),
    ("cancelled", "{n}: request cancelled"),
    ("sent", "Request sent for {n}."),
    ("opened", "Already on this machine: {l}"),
    ("none_present", "No catalog application is on this machine yet."),
    ("empty", "No application offered for this machine."),
    ("no_match", "No application matches."),
    ("unreachable", "The CroustyPatch agent does not answer on this machine ({e})."),
    ("command_badge", "command"),
    ("tab_apps", "Applications"),
    ("tab_updates", "Updates"),
    ("upd_title", "Updates for this computer"),
    ("upd_sub", "{n} update(s) available{sec}"),
    ("upd_sec", ", {s} of them security"),
    ("upd_none", "This computer is up to date."),
    ("upd_check", "Check"),
    ("upd_checking", "Checking…"),
    ("upd_go", "Update ({n})"),
    ("upd_checked", "Last check: {d}"),
    ("upd_managed", "Updates for this computer are managed by your administrator: you can see them here, but not start them."),
    ("upd_reboot", "A restart is needed to finish the updates."),
    ("upd_security", "Security"),
    ("confirm_upd", "Update {n} package(s)?"),
    ("upd_body", "The agent installs them with a restore point and rolls back if something goes wrong. The computer may ask for a restart afterwards."),
    ("upd_running", "Updating the system"),
    ("ok_upd", "Updates installed"),
    ("fail_upd", "The update failed: {m}"),
    ("upd_sent", "Update requested ({n} package(s))."),
    ("check_started", "Looking for updates…"),
    ("check_done", "Check finished: {n} update(s) available."),
    ("upd_unavailable", "Updates are not available on this computer ({e})."),
    ("theme_system", "Theme: follow the system"),
    ("theme_light", "Theme: light"),
    ("theme_dark", "Theme: dark"),
    ("lang_title", "Language: English (click for French)"),
    ("filter", "Filter"),
    ("f_all", "All"),
    ("f_installed", "Installed"),
    ("f_updates", "Needs update"),
    ("f_available", "Not installed"),
    ("sort", "Sort"),
    ("sort_name", "Name"),
    ("sort_status", "Status"),
    ("updates_n", "{n} to update"),
    ("empty_filter", "No application in this filter."),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_languages_have_the_same_keys() {
        let k = |t: &'static [(&'static str, &'static str)]| t.iter().map(|(k, _)| *k).collect::<Vec<_>>();
        assert_eq!(k(FR), k(EN));
    }

    #[test]
    fn placeholders_are_replaced_and_unknown_keys_show_themselves() {
        assert_eq!(tr(Lang::En, "confirm_install", &[("n", "htop".into())]), "Install htop?");
        assert_eq!(tr(Lang::Fr, "confirm_install", &[("n", "htop".into())]), "Installer htop ?");
        assert_eq!(tr(Lang::En, "nope", &[]), "nope");
    }

    #[test]
    fn the_language_follows_the_locale() {
        assert_eq!(Lang::from_locale("fr_FR.UTF-8"), Lang::Fr);
        assert_eq!(Lang::from_locale("en_US.UTF-8"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
    }
}
