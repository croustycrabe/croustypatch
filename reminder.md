# CroustyPatch: where we are (2026-11-08, evening)

Read this first when resuming, then delete it (as the manager asked last time).

## Deployed

- Live: `https://patchmon.lan.croustylab.org` (Zoraxy 10.0.15.10 → docker-admin 10.0.10.16:3000, stack
  `/opt/stacks/croustypatch`, SSH `aiagent` with `~/.ssh/aiagent_ed25519`).
- Image **`croustypatch-server:0.3.2`** since 2026-11-08 19:16 (phase two + terminal app center).
- Backups in the stack dir: `backup-before-0.3.2-20261108-1916.sql.gz` (latest), plus the 0.3.0, 0.2.5, 0.2.4, 0.2.3,
  0.2.2 and 0.2.0 ones. Images 0.3.1, 0.3.0 and 0.2.5 are on the host for rollback: set `CROUSTYPATCH_IMAGE` in `.env`
  and `docker compose up -d`.
- Deploy procedure: build `docker build -f docker/Dockerfile -t croustypatch-server:<v> .` → `docker image prune -f`
  (the dev disk fills up) → pg_dump backup → `docker save | gzip | ssh … docker load` → switch `.env` **only after**
  `docker image inspect` succeeds on the host → `docker compose up -d` → check login/health through Zoraxy.
- Test host 10.0.20.26 (`docker-data`, Proxmox VM 206): agent 0.3.2 with the unsandboxed unit, TUI socket working.
  The manager has switched the app center on and added a catalog app ("Dev").
- Homelab extra: Proxmox LXC 107 `ollama` (10.0.10.40:11434, VLAN 10), model `croustypatch-risk` (qwen2.5:3b,
  `num_thread 4`). Reachable from docker-admin, not from the dev machine. Hypervisor: `ssh aiagent@10.0.10.2`,
  `sudo qm guest exec <vmid> -- …` works for VMs.

## What exists (phase one + two)

Server + agent (Linux apt/pacman, FreeBSD pkg) + web UI. Phase two, all deployed:

1. UI on the **CroustyLab Software** design system, light/dark (`docs/ui.md`).
2. **Notifications** (SMTP, HMAC webhook, ntfy, Gotify; rules, digests, history) and **reboot** module (forced or by
   policy, downtime window, overdue alarm) (`docs/notifications.md`).
3. **AI** contextual risk via OpenAI-compatible API, with guardrail (may lower risk one level at most) and fallback;
   "Ask the AI" on tasks; **MCP** endpoint `/api/v1/mcp` with personal API tokens `cpat_…` (`docs/ai.md`).
4. **Docker** (compose services: pull/up, health, rollback by retag), **helper-script** detection, **OpenSCAP**
   audits, module switches (`docs/modules.md`).
5. **App center**: web portal + catalog, and the **terminal app center** `croustypatch-portal` (ratatui) for servers:
   local presence check (Présent / Partiel / Disponible in colour), progress bar, notification log; talks to the
   agent socket `/run/croustypatch/portal.sock`, the agent relays with its own identity.

Tests: 135 Rust tests (`cargo test --workspace`, needs the dev Postgres `docker compose -f docker/docker-compose.yml
up -d`), 75 browser checks (`cd frontend && npm run build && npm run e2e`, needs `npx playwright install chromium`).

## Open / for the next review

- Live server still needs configuring by the manager: notification channels and rules, AI provider
  (`http://10.0.10.40:11434/v1`, model `croustypatch-risk`), Docker/OpenSCAP module switches, more catalog apps.
- Other hosts must re-run the installer one-liner (no token needed when enrolled) to get the new agent (unsandboxed
  unit, Docker/OpenSCAP modules, TUI). PatchMon's 13 hosts still have no CroustyPatch agent.
- Not built yet: Proxmox API integration (only helper-script detection), scheduled compliance audits (manual/API
  only), app-center script/archive recipes (need signing), desktop GUI client, OIDC/SSO, Let's Encrypt, Wazuh
  integration, dnf backend, full eco mode, FreeBSD agent binary (must be built on FreeBSD).
- The 3B model reasons poorly (contradicted its input in testing): recommend 7B+ / GPU for real decisions.
- Defaults chosen alone, to confirm: digest hour in UTC; Docker updates only with `security_only: false`; app center
  uses the `tasks:create` permission; TUI socket open to every local user unless `portal_group` is set.
- Zoraxy still accepts TLS 1.2 and sends no HSTS (manager's proxy).

## Pitfalls already hit (dev machine)

- Disk fills up with Docker build layers: `docker image prune -f` after each build.
- zsh: `pkill -f`/`pgrep -f` match the calling shell; select by `pgrep -x croustypatch-ag` and check `/proc/<pid>/cmdline`.
- Unix socket paths are limited to 108 bytes: the scratchpad path is too long for test sockets.
- e2e demo hosts rename themselves (`crshell`) after tasks: pick hosts by status, not by name.
