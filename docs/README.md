# CroustyPatch documentation

CroustyPatch is a patch-management and automated-remediation platform (Croustylab). It has three parts:

| Part | Where | What it does |
|---|---|---|
| Server | `server/` (Rust, Actix, Diesel, PostgreSQL) | REST API, WebSocket gateway, policy engine, scheduler, audit log; also serves the web UI |
| Agent | `agent/` (Rust) | Runs as root on Linux and FreeBSD hosts: inventory, patching, rollback, quarantine |
| Web UI | `frontend/` (Next.js, static export) | Dashboard, hosts, policies, approvals, waivers, audit, accounts |
| Shared | `common/` | Wire protocol, signed commands, JWT, crypto primitives |

The requirements live in `specs/` (French). This folder documents what is built.

- [install.md](install.md): install the server, enrol agents, TLS and reverse-proxy modes, backups.
- [policies.md](policies.md): the policy JSON, the rule language, decisions, windows, rollback, waivers.
- [security.md](security.md): how the system is protected, and what it does not (yet) cover.
- [testing.md](testing.md): how the tests are organised and how to run them.
- [ui.md](ui.md): the interface: structure, design decisions and how they relate to the brand.

## What milestone 1 contains

Done and verified end to end:

- Enrolment with one-time tokens; per-agent RSA identity; TLS 1.3 only; certificate pinning for private PKI.
- Two communication modes: persistent WebSocket tunnel (pull) and periodic HTTPS telemetry (push).
- Inventory and pending updates for apt, pacman and FreeBSD `pkg`.
- Policy engine: deterministic rule language, contextual and stability risk, `AUTO_DEPLOY` / `REQUIRE_APPROVAL` / `BLOCK`, maintenance windows, rings, waivers with dual approval.
- Remediation pipeline: server-signed command, restore point, apply, soak with health probes, automatic rollback, and the double-failure protocol (network quarantine and frozen evidence).
- Hash-chained, append-only audit log with verification.
- Accounts, roles and RBAC; bootstrap super-admin with a CLI switch to disable it.
- One-command **Add agent** (installer script and checksummed binaries served by the server, with a wizard that confirms the connection).
- Fleet-wide package view, hosts table with filters and bulk actions, ⌘K command palette.
- Customisable dashboard; bilingual UI (French default, English).

Not built yet (milestone 2): AI provider and MCP, reboot module, notifications (email/SMS/webhooks),
Docker and Proxmox helper-script modules, OpenSCAP compliance, application portal, full eco-mode,
OIDC/SSO and group mapping, Let's Encrypt, Wazuh ingestion/export, dnf/yum. The settings page lists
these modules as "soon"; their switches are stored but have no effect yet.

Phase two:

- [notifications.md](notifications.md): notification channels and rules, the reboot module.
- [ai.md](ai.md): AI-assisted risk, the MCP server and personal API tokens.
- [modules.md](modules.md): Docker, Proxmox helper scripts, OpenSCAP, the app center.
- [ui.md](ui.md): the CroustyLab Software design system, light and dark themes.
