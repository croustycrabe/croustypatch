<p align="center">
  <img src="frontend/public/brand/crab-logo.svg" alt="CroustyPatch" width="120">
</p>

# CroustyPatch

Self-hosted patch management and automated remediation for Linux fleets. A server and a web UI decide what gets
patched, when and how; a small agent on each host applies it, checks that the host is still healthy and rolls back
on its own if it is not.

> Status: pre-1.0. It runs in production on the author's homelab, but expect rough edges and breaking changes.

## What it does

- **Patching with guard rails**: a deterministic policy engine (risk, maintenance windows, rings, approvals, waivers),
  a restore point before every change, a soak period with health probes, automatic rollback, and network quarantine
  with frozen evidence when a rollback fails too.
- **Docker hosts**: containers and compose stacks, newer image versions suggested (and appliable by editing the
  compose file), a migration console for database major upgrades, local or distant registries.
- **App center**: installable and updatable apps per host, in the web UI, a terminal UI and a desktop app.
- **Two ways to talk to hosts**: a persistent WebSocket tunnel (pull) or periodic HTTPS telemetry (push), TLS 1.3
  only, enrolment with one-time tokens and per-agent RSA identities, server-signed commands.
- **Operations**: roles and RBAC, a hash-chained audit log, e-mail and webhook notifications, scheduled reports,
  optional AI second opinion on risk (bring your own model), an MCP server, an offline (air-gapped) mode.
- **Supported hosts**: Linux with `apt` (Debian, Ubuntu) and `pacman` (Arch).

## Roadmap

Next: a native UI for the app center, more Linux distributions (the Red Hat family with `dnf`, Fedora and others),
ARM64 builds alongside amd64, and bringing FreeBSD back.

## Quickstart

You need Docker with the Compose plugin and `openssl`.

```sh
git clone https://github.com/croustycrabe/croustypatch.git
cd croustypatch/deploy
cp .env.example .env && chmod 600 .env
# edit .env (secrets and settings are explained inside), then:
docker compose up -d
docker compose logs server | head -30   # the first-login password is printed once
```

Then open the web UI, change the admin password, use **Add agent** to enrol your first host. The full walkthrough
(TLS and reverse proxy, upgrades, backup and restore) is in [deploy/README.md](deploy/README.md).

The server image is published to GitHub Container Registry as `ghcr.io/croustycrabe/croustypatch-server`.

## Documentation

- [docs/](docs/README.md): installation, policies, security model, notifications, AI, modules, testing, UI.
- [specs/](specs): the original requirements (in French).
- [CHANGELOG.md](CHANGELOG.md): what changed in each release.

## Repository layout

| Path | Content |
|---|---|
| `server/` | REST API, WebSocket gateway, policy engine, scheduler (Rust, Actix, Diesel, PostgreSQL) |
| `agent/` | The agent that runs as root on each host (Rust) |
| `common/` | Wire protocol, signed commands, crypto primitives |
| `frontend/` | Web UI (Next.js, static export, served by the server) |
| `portal-gui/` | Desktop app center (Tauri, built in Docker) |
| `deploy/` | Docker Compose stack for self-hosting |
| `docker/` | Image build and development compose files |

## Development

```sh
docker compose -f docker/docker-compose.yml up -d      # development PostgreSQL on port 5433
cargo test --workspace                                 # needs that database
cd frontend && npm ci && npm run build && npm run e2e  # browser tests (Playwright)
```

See [docs/testing.md](docs/testing.md) for details.

## Security

Please report vulnerabilities privately through GitHub's security advisories for this repository rather than in a
public issue. How the system is protected, and what it does not cover yet, is described in
[docs/security.md](docs/security.md).

## License

[MIT](LICENSE).
