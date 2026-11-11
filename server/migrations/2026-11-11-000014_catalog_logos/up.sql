-- Applications added to the catalog before 0.7 (from the suggestions, or by hand) kept a generic icon: only the suggestions
-- showed the real logo. An app whose name is that of a suggested application, and that still has a generic icon, takes
-- the suggestion's logo. Apps with a logo or an uploaded image of their own are left alone, and so is any app named
-- otherwise: the administrator can still choose another icon in the catalog editor.
UPDATE portal_apps SET icon = 'brand:git', updated_at = now() WHERE lower(name) = lower('Git') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:python', updated_at = now() WHERE lower(name) = lower('Python 3') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:nodedotjs', updated_at = now() WHERE lower(name) = lower('Node.js') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:go', updated_at = now() WHERE lower(name) = lower('Go') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:rust', updated_at = now() WHERE lower(name) = lower('Rust') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:neovim', updated_at = now() WHERE lower(name) = lower('Neovim') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:vim', updated_at = now() WHERE lower(name) = lower('Vim') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:sqlite', updated_at = now() WHERE lower(name) = lower('SQLite') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:postgresql', updated_at = now() WHERE lower(name) = lower('Client PostgreSQL') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:mariadb', updated_at = now() WHERE lower(name) = lower('Client MariaDB') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:htop', updated_at = now() WHERE lower(name) = lower('htop') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:tmux', updated_at = now() WHERE lower(name) = lower('tmux') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:zsh', updated_at = now() WHERE lower(name) = lower('Zsh') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:borgbackup', updated_at = now() WHERE lower(name) = lower('BorgBackup') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:wireshark', updated_at = now() WHERE lower(name) = lower('Wireshark') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:wireguard', updated_at = now() WHERE lower(name) = lower('WireGuard') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:docker', updated_at = now() WHERE lower(name) = lower('Docker') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:podman', updated_at = now() WHERE lower(name) = lower('Podman') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:ansible', updated_at = now() WHERE lower(name) = lower('Ansible') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:keepassxc', updated_at = now() WHERE lower(name) = lower('KeePassXC') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:firefox', updated_at = now() WHERE lower(name) = lower('Firefox') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:thunderbird', updated_at = now() WHERE lower(name) = lower('Thunderbird') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:filezilla', updated_at = now() WHERE lower(name) = lower('FileZilla') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:libreoffice', updated_at = now() WHERE lower(name) = lower('LibreOffice') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:kde', updated_at = now() WHERE lower(name) = lower('Okular') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:gimp', updated_at = now() WHERE lower(name) = lower('GIMP') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:inkscape', updated_at = now() WHERE lower(name) = lower('Inkscape') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:krita', updated_at = now() WHERE lower(name) = lower('Krita') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:vlcmediaplayer', updated_at = now() WHERE lower(name) = lower('VLC') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:audacity', updated_at = now() WHERE lower(name) = lower('Audacity') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
UPDATE portal_apps SET icon = 'brand:obsstudio', updated_at = now() WHERE lower(name) = lower('OBS Studio') AND icon NOT LIKE 'brand:%' AND icon NOT LIKE 'data:%';
