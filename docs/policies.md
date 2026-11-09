# Policies

A policy says which hosts it covers (through groups), how risky an update is, what to do about it, and
how to watch the result. Policies are edited in the UI (**politiques**) or through the API; the stored
form is JSON.

Hosts are matched by group: **a policy with no group applies to no host**. When several policies match a
host, the one with the lowest `priority` number whose `target_scope` is true wins.

## Reference

```json
{
  "security_only": true,
  "target_scope": "asset_environment == 'production'",
  "ai_mode_enabled": false,
  "rules": [
    { "rule_id": "rule_no_ai_01",
      "condition": "cvss_score >= 8.0 AND asset_environment == 'production' AND port_exposed_to_internet == true",
      "contextual_risk": "Critical" }
  ],
  "decisions": [
    { "conditions": { "contextual_risk": "Critical", "stability_risk": "Low" }, "action": "AUTO_DEPLOY" },
    { "conditions": { "contextual_risk_min": "High", "stability_risk_max": "Medium" }, "action": "REQUIRE_APPROVAL" }
  ],
  "default_action": "REQUIRE_APPROVAL",
  "maintenance_window": { "days": [5, 6], "start": "02:00", "end": "05:00", "utc_offset_minutes": 60 },
  "rings": { "delay_minutes": 60 },
  "rollback": {
    "soak_minutes": 30, "auto_rollback": true, "snapshot_required": true,
    "probes": [ { "type": "tcp", "host": "127.0.0.1", "port": 443 },
                { "type": "http", "url": "http://127.0.0.1/health", "expect_status": 200 },
                { "type": "systemd", "unit": "nginx.service" } ]
  }
}
```

`fallback_contextual_risk` (the name used in the spec) is accepted as an alias of `contextual_risk`.

## How a decision is made

1. **Candidates**: pending updates of the host (security only when `security_only`), minus anything covered by an approved, unexpired waiver.
2. **Contextual risk**: the first matching rule sets it. With no match it derives from the highest CVSS (≥9 Critical, ≥7 High, ≥4 Medium), raises it to at least High for a known-exploited CVE, and one level for an internet-exposed host. A security update with no scored CVE is assumed CVSS 5.0 and says so.
3. **Stability risk** (will the patch break the machine?): points for kernel updates, sensitive packages (libc, systemd, openssl, databases, Docker, proxies…), business criticality, production, missing staging twin, large batches; −1 with a staging twin. 0–1 Low, 2–3 Medium, 4–5 High, 6+ Critical.
4. **Decision**: the first decision whose conditions match `(contextual, stability)`; otherwise `default_action`. `contextual_risk`/`stability_risk` match exactly; `contextual_risk_min` and `stability_risk_max` are inclusive bounds.
5. **Task**: `AUTO_DEPLOY` queues a task (scheduled at the next maintenance window, plus `ring × delay_minutes`); `REQUIRE_APPROVAL` waits for an operator; `BLOCK` records a blocked task. The same batch (same packages and versions) is never planned twice, so a rejected, blocked, failed or rolled-back batch does not loop; a newer version is a new batch.

Every task keeps the full reasoning, visible in the UI under the task's detail. `ai_mode_enabled` is
accepted and stored, but until the AI provider exists the deterministic engine is used and the task says so.

## Rule language

Boolean expressions with `== != < <= > >=`, `in`, `AND`/`OR`/`NOT` (or `&& || !`) and parentheses; strings
in quotes, string comparison ignores case. An unknown variable never matches (and never raises an error).
Syntax errors are reported with their position when you save.

Variables: `cvss_score`, `exploited`, `security_count`, `package_count`, `includes_kernel`,
`asset_environment` (alias `environment`), `asset_type`, `criticality`, `os_family`, `distro`, `hostname`,
`port_exposed_to_internet`, `has_staging`, `ring`, `tags` (use `'pci' in tags`), `has_docker_module`,
`has_helper_script_module`.

## Maintenance windows, rings, timeboxing

The window is weekly (`days` 0 = Monday … 6 = Sunday, empty = every day) in a fixed UTC offset; overnight
windows (`22:00`–`02:00`) work. Outside the window an auto-deploy task waits, queued, for the next opening.
Rings stagger hosts: ring N starts N × `delay_minutes` after ring 0 (there is no automatic halt when an
earlier ring fails; watch the tasks). Hosts without a staging twin and with high criticality always require a
restore point, whatever the policy says.

## Rollback and the double-failure protocol

Before applying, the agent creates a restore point (and refuses to continue if `snapshot_required` and a
package cannot be restored). After applying, it watches the probes for `soak_minutes`; **two consecutive
failures of the same probe** trigger an automatic rollback, after which the probes must pass again. A probe
that was already failing *before* the patch is ignored, and if none is left to watch the soak is skipped (the
log says so). If the rollback itself fails, or the services stay unhealthy, the agent isolates the host from
the network (only the management server stays reachable), freezes evidence (logs, package state, restore
point, with a SHA-256 manifest) and reports `quarantined`. An operator lifts it with **rétablir le réseau**.

## Waivers

A waiver defers patching for a package or CVE on a host or group. It needs a justification, compensating
controls, an expiry within 90 days, and two approvals by different people: one holding
`waivers:approve_ciso`, one `waivers:approve_business`; the requester cannot approve their own. Approved,
unexpired waivers are honoured by the planner; revoking one makes the planner act again.

## Operator actions

Scans, patches and rollbacks can also be started by hand. A manual patch is an explicit human decision: it
bypasses the maintenance window and approval step (it is audited), but never targets a quarantined host.
