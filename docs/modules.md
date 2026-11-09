# Optional modules: Docker, Proxmox helper scripts, OpenSCAP, app center, eco mode

Each module is **detected by the agent** (reported in the host's facts) and **switched on or off per host** on the
server. While a module is off for a host, the server drops its data from that host's inventory, refuses its actions
(compliance audit, app center) and nothing is planned for it. No agent configuration is needed.

Where the switches live:

- **Add agent** wizard: the modules ticked there are stored on the enrolment token and switched on for every host
  enrolled with it.
- **Host page > Settings > Modules**: one switch per module, with a *detected / not detected* tag for the ones the
  agent can see (Docker, helper scripts, OpenSCAP). `PATCH /api/v1/hosts/{id}` with `{"modules": {"docker": true}}`
  changes only the modules it names; every change is audited (`host.modules_changed`, before and after).
- **Settings > Modules**: the defaults for hosts enrolled with a token that names no modules. Notifications stay a
  server-wide switch.

When upgrading to 0.4, every existing host starts with the switches that were global until then, so nothing changes
until a host is edited.

Agents from 0.4 receive their switches (WebSocket message `settings`, or `settings` in the push-mode telemetry reply)
and keep the last ones in `settings.json` in their state directory. They then skip the Docker registry checks and
the helper-script scan when those modules are off, and the eco switch replaces the local `eco_enabled` setting. Older
agents keep collecting everything (the server still filters it) until they are updated.

## Agent version and updates

Agents advertise what they understand in their facts (`capabilities`: `settings`, `app_command`, `self_update`), so
the server never sends an order an older agent would misread. The host page shows the agent's version next to the
server's and offers **Update agent** (permission `hosts:write`, `POST /api/v1/hosts/{id}/agent-update`) when they
differ: the signed order names the platform build and its SHA-256; the agent downloads it from its server, checks the
hash, makes sure the new binary starts and prints the expected version, swaps it in place and exits so the service
manager restarts it. The task closes when the new agent starts. Any failure leaves the running binary untouched.
Agents before 0.4 cannot update themselves: re-run the installer one-liner on them once (no token needed).

## Docker

Detected when `/var/run/docker.sock` and the `docker` CLI exist. Every running container is reported as a package
named `docker:<container>` (source `docker`), with its image reference and digest. The agent asks the image's
registry (anonymous token flow: Docker Hub, GHCR, Quay, most self-hosted registries) which digest the tag points to
now; a different digest is an available update. Digest-pinned images are never updated.

Container updates go through the normal pipeline (policy with `security_only: false`, risk, approval, maintenance
window, health probes) and are applied to **Docker Compose services only**:

1. restore point: the image ID each container runs now;
2. `docker compose … pull <service>` then `up -d --no-deps <service>` (volumes, variables, networks and ports stay as
   written in the compose file);
3. wait until the container is running (and *healthy* if it has a health check), then run the policy's probes;
4. on failure: re-tag the previous image and `up -d` again; if that fails too, the double-failure protocol applies.

**Stacks deployed by a manager running in a container** (Arcane, Dockge, Komodo…): compose records the files as the
manager sees them, e.g. `/app/data/projects/hoppscotch/compose.yaml`, which does not exist on the host. The agent
maps such paths through the mounts of the host's containers (here `/app/data` →
`/var/lib/docker/volumes/arcane-agent_arcane-data/_data`) and logs the translation. If no file is found, the update
is refused before anything is touched, with that reason.

Only what was changed is rolled back: a failure before a container was recreated (a pull that fails, a registry that
is down) ends the task as *failed, nothing was changed*, never as a double failure. And a rollback whose command
reports an error, while every container runs its previous image again, counts as rolled back: the state of the
machine decides, not an exit code.

Containers started by hand are reported with their available update but never recreated: the agent cannot know
which of their settings are intentional. Tested read-only on the `docker-data` host (18 compose containers,
registry checks for Docker Hub and GHCR, 12 s).

## Proxmox VE helper scripts

Applications installed by the community helper scripts leave `/opt/<app>_version.txt`; they are reported as
`helper:<app>` with their version. Their update command is interactive, so CroustyPatch reports them and does not
run it. (Proxmox host integration through its API is not part of this release.)

## OpenSCAP

Needs `oscap` and the SCAP Security Guide content (`openscap-scanner` + `ssg-debderived` on Debian/Ubuntu,
`scap-security-guide` on RHEL-like systems). *Run audit* on a host's **Compliance** tab, or `POST
/api/v1/hosts/{id}/compliance-scan` with an optional XCCDF profile id (default: *Settings > Compliance*, else the
SSG `standard` profile).

The agent runs `oscap xccdf eval` under `nice -n 19 ionice -c 3`, keeps the raw result file on the host and reports
the counts, the score (passed / (passed + failed)), the failed rules with their severity and the SHA-256 of the
result file. Reports are append-only in the database (an update trigger refuses edits). A score below the threshold
(default 80 %) raises the `compliance.score_low` event (high), which notification rules can route to the SIEM
webhook or e-mail. The dashboard shows the fleet score (mean of each host's latest report).

## App center (portal)

The *App center* module is switched on per host. **App center > Catalog** (permission `settings:write`) lists the
applications: name, version, category, description, icon, how to install it on each system, and the host groups that
see it (none = every host).

**Installation per system.** An application has one row per package manager (`apt`, `pacman`, `pkg`, `dnf`) and/or a
row for every system (`*`), used by hosts without a row of their own. A host whose package manager matches no row does
not see the application. This is how a tool named `fd-find` on Debian and `fd` on Arch is one catalog entry. Each row
is either:

- **packages**: native package names, installed and removed by the agent's package manager;
- **command** (internal tools, vendor installers): an *install* and an *uninstall* command and a *checked path*. The
  agent runs the command as root with `sh -c`, `umask 022`, a cleared environment (`PATH` limited to the system
  directories, `HOME=/root`) and a 30-minute timeout. Nothing from the requester is ever put into the command; the
  requesting local account is given as variables only: `CROUSTYPATCH_USER`, `CROUSTYPATCH_USER_HOME`,
  `CROUSTYPATCH_USER_UID`, `CROUSTYPATCH_USER_GID` (absent for a web request). After an install the agent checks that
  the path exists **and is usable by the user who asked**: every directory on the way searchable, the file executable
  (or readable). A request from the web UI must be usable by every account. An app that ends up root-only is reported
  as a failure. After an uninstall the path must be gone.

A command runs as root on every host in scope, so **writing, changing or deleting an app with a command needs the
`portal:commands` permission** (only *super_admin* has it by default; give it in a custom role). The full command
text is in the audit log and in the signed order, and the terminal app center shows it before asking for
confirmation. It is stored and shown in clear (catalog, audit log, every local user of the terminal app center):
**never put a password or token in a command**; have the command fetch secrets from where the host already keeps
them. Command apps are only offered for install on agents 0.4 and later.

**Suggestions** (tab *Suggestions*, `GET /api/v1/portal/suggestions`): 46 common applications (development, system,
backup, network, containers, security, desktop) with their apt and pacman package names, each checked against the
Arch repositories and Debian 13 (all present on Debian 12 except `fastfetch`). *Add* copies one into the catalog,
where it can be edited and scoped.

**App center > Applications** shows, for a chosen machine, the apps offered to it, whether each is installed (all
its packages present) and any operation in progress. *Install* / *Uninstall* (`POST /api/v1/portal/install` with
`host_id`, `app_id`, `action` = `INSTALL` | `UNINSTALL`, permission `tasks:create`) creates an `app` task; the server
checks that the app is enabled and in the host's scope, signs the order, and the agent installs or removes the
packages as root, then reports the new inventory. One app operation at a time per host; every request is audited.

### Terminal app center (servers without a desktop)

On any host with the agent, a local user without root runs **`croustypatch-portal`** (a link the installer creates to
the agent binary; also `croustypatch-agent portal`). It is a full-screen terminal interface (ratatui): categories on
the left, the applications on the right, details and a **status and notifications** area below.

- **Presence, checked on the machine itself** every time the list is refreshed (and when the portal opens): green
  *Present* (every package installed, even if it was installed by hand; for a command app, its checked path usable by
  the person at the keyboard), amber *Partial n/m* (the details list how
  many packages are missing; the app can be completed or removed), grey *Available*. On opening, a note lists the
  applications already present.
- **Status area**: a progress bar for the operation in progress (queued → handed to the agent → running, with a
  spinner) and a timestamped log, newest first: ✔ success, ✖ failure with the reason (e.g. the package manager's
  error), ⚠ cancelled, ℹ information. Each change is reported once; operations from before the portal opened are not
  reported again. `c` clears the log.
Keys: ↑/↓ choose · ←/→ category · `i` install · `u` uninstall · `r` refresh · `c` clear notifications · `q` quit; installs and removals ask
for confirmation, and the list follows a running operation. French or English follows `$LANG`.

How it works: the TUI talks to the agent over a Unix socket (`/run/croustypatch/portal.sock`, setting
`portal_socket`). The agent relays to the server **with its own signed identity** (`GET /api/v1/agent/portal`, `POST
/api/v1/agent/portal/install`), so the server only ever lets a host act on itself, with the same checks as the web
portal (module on, app offered to that host, one operation at a time). The local account comes from the socket's
peer credentials (not from the client) and is recorded as `local:<user>@<host>` in the task and the audit log. The
socket is open to every local user (mode 0666), since the catalog is admin-approved; set `portal_group = "<group>"`
in the agent's configuration to restrict it to one group (mode 0660).

### Desktop app center (laptops and workstations)

The same app center in a window (Tauri, `croustypatch-portal-gui`): categories, search, presence checked on the
machine (*Present*, *Partial n/m*, *Available*), a confirmation that shows the packages or the exact command before
anything runs, a progress bar and a notification log. French or English and light or dark follow the system.

- **Who gets it.** The agent tells the installer what the machine is (`croustypatch-agent machine-type`): *laptop*
  (system battery), *workstation* (a display manager or desktop sessions installed) or *server*. Laptops and
  workstations get the window, its menu entry (*CroustyPatch App center*) and icon, and the WebKitGTK runtime it
  needs (`libwebkit2gtk-4.1-0` on Debian/Ubuntu; on Arch the installer asks you to run
  `pacman -Syu webkit2gtk-4.1` rather than risk a partial upgrade). `CROUSTYPATCH_PORTAL_GUI=1` or `=0` forces the
  choice. Hosts are now enrolled as *laptop*, *workstation* or *server* accordingly; hosts enrolled earlier keep
  their type, and the host page flags a *server* on which a desktop is detected.
- **Which one opens.** `croustypatch-portal` opens the window when the user is in a graphical session
  (`WAYLAND_DISPLAY` / `DISPLAY`) and the window is installed, the terminal app center otherwise (for example over
  SSH on a laptop). `--gui` / `--tui` force it.
- **Same rights as the terminal.** The window talks to the same socket; the agent identifies the user from the
  socket, never from the window. The window can call three commands (list, install, uninstall) and nothing else: no
  shell, file or network access, a strict content security policy, fonts and icons bundled.
- **Updates.** *Update agent* on the host page also replaces the window when it is installed (same SHA-256 check;
  both files are verified before either is replaced).
- Built for Linux x86_64 only (Debian 12+, Ubuntu, Arch). FreeBSD and aarch64 machines use the terminal app center.
  If the window stays blank under Wayland with some graphics drivers, start it with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

On the web, a command app shows as installed after its last successful install (the server cannot look at the
machine); the terminal app center checks the machine itself.

Not in this release: a desktop window for FreeBSD and aarch64.
