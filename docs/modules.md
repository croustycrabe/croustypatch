# Optional modules: Docker, Proxmox helper scripts, OpenSCAP

Each module is **detected by the agent** (reported in the host's facts) and **switched on or off on the server**
(*Settings > Modules*). While a module is off, the server drops its data from inventories and nothing is planned for
it. No agent configuration is needed.

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

*Settings > Modules > App center* switches it on. **App center > Catalog** (permission `settings:write`) lists the
applications: name, version, category, description, icon, the **native packages** to install (apt, pacman or pkg) and
the host groups that see it (none = every host).

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
  *Present* (every package installed, even if it was installed by hand), amber *Partial n/m* (the details list how
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

Not in this release: installing from scripts or archives (they would need signed recipes) and a desktop GUI client.
