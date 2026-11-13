# Changelog

All notable changes to CroustyPatch. Versions follow the server, agent and desktop app, which are released together.
The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [0.10.0]

### Added
- Docker: newer-image detection now works on very large registries (successor probing when the tag list is partial,
  `-sha-` tags compared by version, credentials read from `~/.docker/config.json`), with a per-container explanation
  of why no update is suggested.
- Docker: a migration console with templates (PostgreSQL, MySQL, MariaDB, MongoDB, Redis, other, read-only) and a
  live log; the exact commands are audited. New permission `docker:shell`, super administrators only by default.
- Docker: an AI-drafted migration plan, validated server side and always reviewed by a person; environment values are
  never sent to the model.
- Docker registries (Settings): local and distant registries, pull-through mirrors, project path prefix, logins. In
  offline mode only local registries are used and at least one is required to enable the Docker features.
- App center: filters (All, Installed, Needs update, Not installed, with counts) and a sort by status in the web
  portal, the terminal app center and the desktop app.
- `deploy/`: a ready-to-use Docker Compose stack (server and PostgreSQL 17) with `.env.example` and a README.
- GitHub Actions: a workflow that builds the server image and publishes it to GitHub Container Registry on version
  tags, and a CI workflow (clippy, tests against PostgreSQL, web type check and build).
- `LICENSE` (MIT) and this changelog.

### Removed
- **Compliance / OpenSCAP**: the agent module, the audits, reports and notifications, the SCAP content bundled in the
  image and the web pages. Migration `000016` drops the compliance tables and deletes compliance tasks: take a
  database backup before upgrading.
- **FreeBSD** support (`pkg` backend, rc.d installer, `pf` quarantine rules, agent platform). Migration `000018`
  removes the `pkg` key from stored app recipes.
- The homelab-specific deployment files; use `deploy/` instead.

### Changed
- Agents on 0.9.0 keep working with a 0.10.0 server; update them with *Update agent* to get the Docker console,
  registries and the new detection (older agents get a 409 on the console).

## [0.9.0]
- Administration > Logs: a server log view (in-memory ring of recent lines).
- App updates are shown in the web and terminal app centers (installed version against the latest known).
- Debian and Ubuntu SCAP content bundled with the server image (later removed with compliance, see 0.10.0).

## [0.8.0]
- Separate alerts for system updates and Docker updates; clean HTML digest e-mail.
- Offline (air-gapped) mode is pushed to the agents: no registry or AUR lookups.
- Planned audits and per-host audit reporting; OpenSCAP on newer Ubuntu releases (removed in 0.10.0).

## [0.7.x]
- 0.7.6: the agent reopens a dead tunnel after 90 s of silence (suspend, Wi-Fi changes).
- 0.7.5: no database major upgrade through one click, per-container "updating" state, app updates visible.
- 0.7.4: Visual Studio Code from the AUR on Arch, apply a suggested Docker version by editing the compose file.
- 0.7.3: Visual Studio Code and Zed in the app center, server version shown in the interfaces.
- 0.7.2: update the system, Docker, or both; newer Docker versions shown as suggestions.
- 0.7.1: newer Docker versions, logos in the app center, paged package lists.
- 0.7.0: Docker menu, app-center updates and notifications, app icons.

## [0.6.x]
- 0.6.1: host resources and hardware, customisable dashboard, package views, desktop language and theme.
- 0.6.0: patch days that actually run (first-match shadowing fixed), monthly repeat and time zones, scheduled reports,
  mail recipients; policies can include or exclude Docker.

## [0.5.x]
- 0.5.1: compose stacks from containerised managers, no isolation when nothing changed, SSH kept during isolation.
- 0.5.0: desktop app center (Tauri) and machine-type detection.

## [0.4.0]
- Per-host modules, app-center recipes and commands, suggestions, agent self-update.

## [0.1.0] to [0.3.x]
- First milestones: server, agent and web UI; enrolment with one-time tokens; TLS 1.3; pull (WebSocket tunnel) and
  push modes; apt and pacman inventory; policy engine with risk scoring, windows, rings and waivers; remediation
  pipeline with restore point, soak and automatic rollback; hash-chained audit log; roles and RBAC; notifications;
  AI risk assessment and MCP server.
