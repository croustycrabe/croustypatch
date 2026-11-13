# Optional modules: Docker, Proxmox helper scripts, app center, eco mode

Each module is **detected by the agent** (reported in the host's facts) and **switched on or off per host** on the
server. While a module is off for a host, the server drops its data from that host's inventory, refuses its actions
(app center) and nothing is planned for it. No agent configuration is needed.

Where the switches live:

- **Add agent** wizard: the modules ticked there are stored on the enrolment token and switched on for every host
  enrolled with it.
- **Host page > Settings > Modules**: one switch per module, with a *detected / not detected* tag for the ones the
  agent can see (Docker, helper scripts). `PATCH /api/v1/hosts/{id}` with `{"modules": {"docker": true}}`
  changes only the modules it names; every change is audited (`host.modules_changed`, before and after).
- **Settings > Modules**: the defaults for hosts enrolled with a token that names no modules. Notifications stay a
  server-wide switch.

When upgrading to 0.4, every existing host starts with the switches that were global until then, so nothing changes
until a host is edited.

Agents from 0.4 receive their switches (WebSocket message `settings`, or `settings` in the push-mode telemetry reply)
and keep the last ones in `settings.json` in their state directory. They then skip the Docker registry checks and
the helper-script scan when those modules are off, and the eco switch replaces the local `eco_enabled` setting. Older
agents keep collecting everything (the server still filters it) until they are updated.

## Host information (agents 0.6 and later)

Each host page shows what the machine is made of and how busy it is.

- **Hardware** (in the host's facts, refreshed with each inventory): processor model and logical cores, memory and swap,
  manufacturer and model as the firmware reports them (`/sys/class/dmi/id`), BIOS, and what it runs on (`lxc`, `kvm/qemu`,
  `vmware`… or bare metal). No serial numbers or identifiers are collected.
- **Resources** (sent with every heartbeat; an agent that has just connected sends one at once): CPU use since the previous
  reading, memory and swap in use, load averages, uptime and every real filesystem (a device mounted several times is listed once,
  `df -P -k -T` without tmpfs, overlay and squashfs). The server keeps the latest reading and one point every five minutes
  for two weeks: the card shows meters (green below 75 %, amber to 90 %, red above) and 24-hour charts. `GET /api/v1/hosts/{id}/metrics?hours=24`.
- The dashboard summarises the fleet (average CPU and memory, the busiest machines), and hosts that stopped reporting for more
  than 15 minutes drop out of it. Linux only for now: other systems report no resources.

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

### Docker menu (0.7)

The **Docker** page lists every container of every host whose Docker module is on, and each host has a **Docker** tab
with the same list. A row shows the container, its image, the state and health the agent saw, its compose project
and service, and whether a newer image exists (`current` and `candidate` digests). Search matches the container name
or the image; *Updates only* keeps the containers with a newer image. `GET /api/v1/docker?host=&updates=&q=`.

**Newer versions (0.7.1).** A running `v3.3.1` has its own digest checked (an image rebuilt under the same tag), but a
newer *tag* such as `v3.4.0` is a different image. For a container whose tag is a version, the agent also lists the
tags of the repository in its registry and reports the highest one of the **same shape**: `v3.3.1` is compared only with
other `v#.#.#` tags, `17-alpine` only with `#-alpine`, `16-vectorchord1.1.1` with `#-vectorchord#.#.#`. Release
candidates, other flavours, build tags (`…-sha-3699278`) and moving tags (`latest`, `release`) are never offered. A
new first number is marked **major** (and the newest of the running major is named next to it, for a database that
needs a migration). The Docker page and tab show it as *Update suggested*, count it in *Updates suggested* and can
filter on it.

It is shown as a **suggested update** (*v3.3.1 → v3.4.1*). The ordinary *Update* buttons keep pulling the running tag,
because another tag is another image: the compose file has to say so, or the next `docker compose up` would bring the
old one back. See *Apply a suggested version* below. How it is
checked: Docker Hub lists tags alphabetically, GHCR in no order (immich-server has more than 33 000, 34 pages, about
7 s), so the agent reads the pages (at most 100 and 60 s per repository), keeps only tags of the right shape as they
arrive, and stops early only when a page proves the list is alphabetical and past the tag's prefix. The answer is
kept in memory for 6 hours per image (a failed lookup 20 minutes), so a registry is asked a few times a day at most.
Agents from 0.7.1; anonymous access only (public images), like the digest check. `GET /api/v1/docker?newer=true`.

**Apply a suggested version (0.7.4).** A suggestion has a **Move to v3.4.1** button (agents 0.7.4 or later, `tasks:create`).
The agent changes the tag in the service's compose file and recreates it with the usual cycle (pull, `up -d` for that
service, wait until healthy, the policy's probes); if that fails the compose file **and** the previous image are put
back (the policy's *auto rollback*), and a manual rollback of the task does the same. The change is deliberately
narrow, and the agent decides it on the real file, not the server:

- the service has a **literal `image:` line** directly inside its block, and that line says exactly what the container
  runs now (so it is the definition it was created from). With several compose files the last one that defines the
  service is the one edited. Only the tag characters change: indentation, quotes, comments and line endings stay, as do
  the file's mode and owner. A variable (`${VERSION}`), an alias or anchor, a `<<` merge key, `extends`, a build-only
  service, an inline mapping or tabs are **refused with the reason** and the tag is changed by hand (the page says so
  and shows the image to write). On the real stacks of this installation 48 of 51 services qualify.
- the new image is built **by the server from what the agent reported** (the running image with the suggested tag): the
  request names containers (`{"host_ids", "scope": "docker", "packages": ["docker:immich_server"], "to_latest": true}`),
  never an image; the agent checks again that it is the same image with only the tag changed.
- everything is planned before anything is written: if one container of the task cannot be changed, nothing is touched.
  Containers of one file are edited together, and a file that already says the new tag only has its container recreated.
- the original text is kept in the task's restore point; it is put back only if the file still is what the update
  wrote, never over a change somebody made since (the rollback then says so and the old tag is restored by hand).
- a **database**'s major version is **never** applied this way, confirmed or not (`postgres`, `mariadb`, `mysql`, `mongo`, immich's and supabase's Postgres images, ...): the new image cannot open the old data directory, so it needs a dump and restore or `pg_upgrade`. A real attempt on `hoppscotch-db` (17 to 18) did roll back cleanly (file and image put back), which is why it is now refused upfront. Minor and patch versions of a database are fine. The agent says `manual` with that reason and the server refuses a request for it.
- a **major** version (the first number goes up: a database's next major, `v3` to `v4`) is refused unless the request
  says `allow_major`; the page shows a warning about migrations and keeps the button off until it is acknowledged.
- an agent without the `docker_retag` capability is never sent a version change (it would pull the tag it already has
  and report success): the server answers 409 and says to update the agent. The inventory tells the page whether each
  suggestion can be applied (`latest.apply`: `file` with the path, or `manual` with the reason).

The whole cycle was run against a real Docker daemon and compose plugin (`agent/tests/docker_live.rs`, ignored by
default: it needs docker and internet): a version applied (file and container, healthy) and rolled back by hand
(file back byte for byte, previous image healthy); a tag from a variable refused with nothing touched; a tag that
cannot be pulled leaving file and container as they were; a new version that turns unhealthy rolled back with its file.
The planner was also run on sanitised copies of every real compose file of this installation.

**System, Docker, or both (0.7.2).** Starting updates by hand takes a *scope*: the host page and the hosts list's bulk
bar offer **Update** → *System · n*, *Docker · m* or *System and Docker · n+m* (a plain button when only one kind is
pending). `POST /api/v1/tasks` takes `"scope": "system" | "docker" | "all"` (default `all`, as before): the system
packages and the container images stay separate tasks whatever the choice, and the scope only says which are created.
A request for a kind that is already open on the host is refused with the reason, a scope with nothing pending says
so (*no pending container updates*), and with several hosts the ones that have something of that kind get a task.
`GET /api/v1/hosts` adds `docker_updates` (the container images among `updates`). The audit log records the scope.
The app center on a laptop stays system-only.

CroustyPatch only ever **updates images**. It never starts, stops or removes a container: the *Update* buttons queue
a task for the chosen containers (or all of those with an update), and the host's policy decides what happens
around it (see `policies.md`, *Docker*).

Container images are a task of their own (kind `docker`), separate from the system patch of the same host: a
container that fails to restart never rolls a system update back, and a running container update does not hold up
a system patch (and the other way round). The planner, the manual *Update* buttons and the policy preview all split
a batch this way; each kind waits only for its own open task. Agents need no change: both kinds run the same routine.

### Why a container has no suggestion (0.10)

Each container carries what its registry checks found (`meta.check`: `digest` and `tags`), and the Docker page shows it
when there is no suggestion instead of a bare "up to date": *check failed* with the registry's answer (a private image
needs a `docker login` made on the host by root, whose `~/.docker/config.json` the agent reads; credential helpers are
not run), *floating tag* (`latest`, `release`: only the digest is followed), *pinned by digest*, or *offline*. The
possible values of `tags`: `listed` (every tag read), `probed`, `not_version`, `pinned`, `offline`, `error: …`.

Newer versions are found by listing the repository's tags. Some repositories cannot be listed: GHCR lists tags in no
order and `immich-machine-learning` has a tag per commit and per hardware variant (more than 100 pages of 1000), `envoy`
takes more than a minute. Up to 0.9 such a list was cut short and cached for 6 hours as "nothing newer" (immich-machine-learning
v3.3.1 showed no v3.4.1). Since 0.10, when the list is incomplete the agent follows the versions one by one: it asks the
registry (a `HEAD` on the manifest) whether `v3.3.2`, `v3.4.0`, `v4.0.0`… exist (each number raised by 1 to 3, the
following ones set to 0 or left as they were), takes the highest that does and starts again from it, at most 90 requests
and 12 rounds per image. Tags ending in `-sha-<hash>` (`supabase/studio`) are compared by their version. A manual scan
(*Scan now*) clears the agent's 6-hour cache; a failed lookup is retried after 20 minutes. Checked live on the real
images: `immich-machine-learning:v3.3.1` gives `v3.4.1`, `envoy:v1.39.1` gives `v1.40.2`, `supabase/studio:2026.10.12-sha-3699278`
gives `2026.11.13-sha-64fad1a`.

### Migration console (0.10)

When a new tag is not enough (a database's next major: the new image cannot open the old data directory, which is why
those are never applied automatically), the **Migration console** button of a container with a suggested version opens
a window with **prebuilt command lines** for its kind of software, editable, run on the host from the page.

- **Templates** (`GET /api/v1/docker/migration?host_id=&container=&to=`): PostgreSQL (and its variants: PostGIS,
  pgvector, VectorChord, Timescale), MySQL/MariaDB, MongoDB, Redis/Valkey, anything else (backup of the data directories,
  new image, logs) and a read-only *Look first*. Each starts with a **backup** to `/var/backups/croustypatch`, moves
  the old data directory aside as `<dir>.pre-upgrade` and keeps a copy of the compose file as `<file>.pre-upgrade`:
  nothing is deleted, and the rollback lines put both back. Without a compose file the agent can read, a database
  template comes in two parts around the change of image, which the person makes in the tool that manages the stack.
  Tested on a real Docker daemon: PostgreSQL 16 to 17 with its data, then the rollback.
- **Running** (`POST /api/v1/docker/shell`): the steps run one after the other as root with `sh -c` (capability
  `docker_shell`, agents 0.10+, an older one gets a 409), stop at the first one that fails, and the log (each step's
  output, head and tail when long) follows in the window; a heartbeat every minute keeps a long dump from looking like a
  dead agent. One migration at a time per host; the task is visible on the Tasks page. Up to 20 steps of 8000 characters.
- **Permission:** running needs `docker:shell`, which only `super_admin` has by default (the built-in `admin` role does
  not: it has `hosts:*` but nothing in this namespace); everyone who can read hosts sees the templates and can copy
  them. The **exact commands are in the audit log** (`docker.shell_requested`), as sent.
- **Do with AI** (`POST /api/v1/docker/migration-plan`, `docker:shell`): when an AI provider is configured (Settings > AI),
  the provider receives the container, service and project names, the image and the tags, the compose file paths and the
  baseline template (never environment values or file contents) and returns a plan, which is validated (valid JSON,
  at most 12 steps, one-line titles) and shown with its summary, its risks and a tag on every step that deletes files or
  Docker data, drops data, writes to a disk, downloads or restarts something. It is a draft: it fills the same editable
  list, a person reads it, and only then runs it (the confirmation says it comes from the AI). The request is audited
  (`docker.migration_plan_requested`) without its content.

### Registries: local or distant (0.10)

*Settings > Docker registries* (`GET`/`PUT /api/v1/docker/registries`, `settings:write`) lists the registries the agents
use. A registry has a name, a **kind** (`local`: on your network, works offline; `distant`: Docker Hub, GHCR, a private
one), an address (`registry.lan:5000`), the registries it **mirrors** (`docker.io`, `ghcr.io`: a pull-through cache or
a Harbor proxy project), a path prefix (`dockerhub-proxy/`), a login and a plain-HTTP flag for a registry without TLS.
Passwords are stored encrypted (like the AI key), never returned, and an empty password keeps the stored one. Agents
with the Docker module on receive the list with their settings.

- **Lookups:** an image is looked up in a configured registry that mirrors its own (the local cache first), then in its
  own registry (with that registry's login when it is configured, else what `docker login` recorded), so the
  digest and the versions are those of the copy an update would pull. A cache that holds Docker Hub official images
  keeps their `library/` path (`harbor.lan/dockerhub-proxy/library/postgres`).
- **Taking an update from a registry:** the *Apply* dialog has a **Registry** choice (`registry_id` on `POST /tasks` with
  `to_latest`): the new version is pulled from that registry and its address is written into the compose file, so the
  container keeps using it ("the same image and tag, held there": `ghcr.io/x/y:v1.1.0` becomes
  `registry.lan:5000/mirror/x/y:v1.1.0`). Before the pull the agent runs `docker login` for a registry that has a user
  name (the login lands in root's Docker config); a refusal changes nothing. The Docker daemon's own mirror setting
  (`registry-mirrors`) is not touched.
- **Offline (air-gapped) mode:** only `local` registries are used and sent to agents, and a local registry is
  **mandatory** for the Docker features (newer-image detection, container updates, new versions). Without one they
  cannot be switched on, and the server says so with a 400 that names the fix: switching the Docker module on (for the
  fleet in *Settings > Modules* or for one host), choosing offline mode while the Docker module is on, removing the last
  local registry, and `POST /tasks` for a new version (409). The Docker page and the registries card show a notice
  with the same explanation, and a container whose registry no local registry relays says so in its check.

## Proxmox VE helper scripts

Applications installed by the community helper scripts leave `/opt/<app>_version.txt`; they are reported as
`helper:<app>` with their version. Their update command is interactive, so CroustyPatch reports them and does not
run it. (Proxmox host integration through its API is not part of this release.)

## App center (portal)

The *App center* module is switched on per host. **App center > Catalog** (permission `settings:write`) lists the
applications: name, version, category, description, icon, how to install it on each system, and the host groups that
see it (none = every host).

**Installation per system.** An application has one row per package manager (`apt`, `pacman`, `dnf`) and/or a
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
- Built for Linux x86_64 only (Debian 12+, Ubuntu, Arch). aarch64 machines use the terminal app center.
  If the window stays blank under Wayland with some graphics drivers, start it with
  `WEBKIT_DISABLE_DMABUF_RENDERER=1`.

- **Updates from the app center (0.7).** An **Updates** tab next to *Applications* lists the system updates waiting on
  this machine (security ones first) with *Update everything*, *Update selected*, and *Check now* (refreshes the
  repositories and sends a fresh inventory). **No approval step**: the person at the keyboard decides, and the
  request becomes a queued `patch` task that the agent runs like any other (the host's policy still supplies the
  health probes and the rollback options). It is allowed only where the **Updates from the app center** switch
  (`portal_updates`) is on for the host: on by default for **laptops and workstations**, off for servers, per host
  or per enrolment token (*Add agent > App center updates*: automatic, yes, no). The server checks, for every
  request, that the app center is on for the host, that the host is not pending or quarantined, that no patch or
  rollback is already open, and that every name is a **pending system update of that machine** (never a container,
  never an arbitrary package name). The task is recorded as `local:<user>@<host>` and `portal.update_requested` is
  audited. The terminal app center does not have this tab.
- **Notifications.** The installer adds `/etc/xdg/autostart/croustypatch-update-notifier.desktop`: at login the
  window binary runs in `--notifier` mode, with no window. It asks the agent's socket (same rights as the window) every
  30 minutes (`CROUSTYPATCH_NOTIFIER_INTERVAL`, 60 s to 1 day) and, when updates can be installed here, sends a desktop
  notification (`notify-send`, else D-Bus) in the session language. It speaks again only when the list of updates
  changes (fingerprint kept in `$XDG_STATE_HOME/croustypatch/last-notified`), never on servers or on machines where
  the switch is off. The installer adds `libnotify` (`libnotify-bin` on Debian/Ubuntu) when it is missing. A laptop
  enrolled before 0.7 gets the login entry from *Update agent* (the agent adds it right after replacing the window
  binary, never over an existing file) but not `libnotify`: without `notify-send` the notifier falls back to D-Bus
  (`gdbus`), and the installer can be re-run to add the package.
- **Suggested editors (0.7.3, Arch changed in 0.7.4).** *Visual Studio Code* and *Zed* are in the suggestions, and neither
  is in the repositories of Debian or Ubuntu, so those systems install them with a **command** (the first suggestions to
  do so; adding one needs `portal:commands`, the card is flagged *command* and asks for a confirmation that says it runs
  as root; the shipped list names them in `REVIEWED_COMMANDS` and tests read their scripts). **VS Code** on Debian and
  Ubuntu adds Microsoft's apt repository (signed key, `/etc/apt/sources.list.d/vscode.sources`) and installs `code`, so
  its updates arrive with the system updates; removing it removes the repository. **VS Code on Arch** is Microsoft's
  build from the AUR (`visual-studio-code-bin`, which is not in pacman): the recipe installs `base-devel` and `git`,
  clones the AUR repository and builds it as the unprivileged user `nobody` (`makepkg` refuses to run as root, and
  `--nodeps` leaves the runtime libraries to `pacman -U`), then installs the package with pacman. The AUR is
  community-maintained and unreviewed, so the recipe checks what it is about to run: only the expected files, the
  root-run install script and the launcher pinned by SHA-256 (read on 2026-11-11), sources only from
  `update.code.visualstudio.com` and a few suspicious words refused; if the package changes, the install stops with the
  reason and the recipe has to be reviewed. It refuses to run when Code - OSS is installed (the two conflict) and
  there is no automatic update, but the app center **says when one is available** (desktop window 0.7.5; terminal app center and web app center 0.9: the server sends Arch agents the AUR packages named by the catalog, `watch_aur`, the agent checks them with each inventory and reports `app_updates`; the TUI shows *↑ Update 1.146.0* and `i` updates, the web App center shows the update with an **Update** button): a command recipe can name where its latest version is published (`"latest": {"aur": "visual-studio-code-bin"}`), the agent compares pacman's installed version with the AUR's (`vercmp`, asked at most hourly) and the card shows *Update 1.146.0* with an **Update** button, which runs the install again (it rebuilds the latest). Software installed by a command is otherwise invisible to the package manager's update list. Apps already in the catalog keep the recipe they were added with: remove and add VS Code again to get the new one. The open-source build is its own suggestion,
  **Code - OSS** (Arch only: the repository package `code`; Debian and Ubuntu have none). **Zed** on Debian and Ubuntu uses
  Zed's official release archive (the one its own installer downloads), read completely before anything is replaced,
  unpacked as `root:root` into `/opt/zed.app` with `/usr/local/bin/zed`, a menu entry and an icon; it has no
  automatic update (install again to get the latest). Arch installs `zed` from `extra`. The scripts were run on clean
  Debian 13, Ubuntu 26.04 and Arch containers (install, install again, uninstall; Arch also refusing a changed AUR
  script and Code - OSS in the way). Zed needs a graphical desktop and a Vulkan graphics driver and refuses to run as
  root.
- **App icons (0.7).** Applications added before 0.7 kept a generic icon (only the suggestions showed the logos): 0.7.1 gives
  those that carry the name of a suggested application (`git`, `Firefox`, `Python 3`…) its logo, once, and a new
  application left with the default icon takes the logo of the application it is named after. An icon chosen on
  purpose, a logo or an uploaded image is never replaced, and the catalog table now shows each icon.
  The rest as in 0.7: An app's icon is a generic glyph, a **logo** from the bundled collection (121 icons from
  [Simple Icons](https://simpleicons.org), CC0, in `server/assets/brand-icons.json`, rebuilt by
  `server/assets/brand-icons/generate.py`; the suggested apps use them where one exists), or an **uploaded image**
  (PNG, JPEG, WebP or SVG; rasters are scaled to 128 px in the browser, SVG up to 64 KB; at most about 72 KB stored).
  The server stores `brand:<name>` or a `data:` URI, resolves logos to an image (`icon_url`) and checks uploads by
  content, not by label (magic bytes; an SVG with a script, event handler, foreign object or entity is refused). Images
  are only displayed through `<img>`, where an SVG cannot run scripts, and nothing is fetched from the internet.

On the web, a command app shows as installed after its last successful install (the server cannot look at the
machine); the terminal app center checks the machine itself.

Not in this release: a desktop window for aarch64, and updates or notifications in the terminal app center.
