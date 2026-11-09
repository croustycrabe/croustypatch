# Notifications and reboot

## Notifications

**Notifications** in the sidebar (permission `settings:read` to view, `settings:write` to change).

- **Channels** say where messages go: e-mail (SMTP with STARTTLS, implicit TLS or none), a **signed webhook**, **ntfy**
  or **Gotify**. Passwords, HMAC keys and tokens are stored AES-256-GCM encrypted and are never returned by the API.
  "Send test" delivers a test message right away and shows the error if it fails.
- **Rules** say what and to whom: a list of events, a minimum severity, one or more channels, an optional scope
  (host groups), and for e-mail the recipient roles plus extra addresses. A rule is **immediate** or a **daily
  digest** sent at a chosen UTC hour (or with "Send digest now").
- **History** lists every delivery with its status; failed deliveries are retried five times with growing delays.

Events and default severity: host quarantined, host unresponsive after planned downtime, host not back after reboot
(critical); task failed, patch rolled back (high); host offline, patch awaiting approval (medium); patch applied,
reboot scheduled / started / completed (info). Successful scans are not notified.

Events are queued in the same database transaction as their audit entry, so nothing recorded is lost and nothing is
sent for an action that was rolled back.

### Webhook signature

```
POST <url>
content-type: application/json
x-croustypatch-event: host.quarantined
x-croustypatch-delivery: <uuid>
x-croustypatch-timestamp: 1762610000
x-croustypatch-signature: sha256=<hex HMAC-SHA256(key, "<timestamp>.<raw body>")>

{"id": "...", "event": "host.quarantined", "severity": "critical", "title": "...", "text": "...", "data": {...}}
```

Verify the signature over the raw body and reject old timestamps to stop replays.

## Reboot

- **Force a reboot** from a host's menu (or the "reboot needed" banner). Servers restart at once; workstations and
  laptops get 10 minutes of warning by default. The warning and the optional message go through the system's
  `shutdown` broadcast.
- **After a patch** that needs a restart, the policy decides (section *Reboot*): do nothing, reboot automatically in
  the maintenance window, or warn users 30 minutes ahead then reboot.
- While a host restarts it is in **planned downtime** (status *maintenance*) for the warning delay plus 15 minutes,
  so supervision does not raise false alarms. When the agent reports from the new boot the task succeeds and the
  host goes back online. If it stays silent past the window, the reboot task fails, the host turns *alert* and the
  critical "host did not come back after reboot" event is sent.
