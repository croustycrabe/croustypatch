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

## Reports

Rules tell you **when something happens**: they queue events and, for a digest, send what accumulated. If nothing
happened, nothing is sent. A **report** is different: a summary of the fleet **built from the database when it is sent**,
on a daily, weekly or monthly schedule, **even when nothing happened** (it then says everything is up to date). Use rules for alarms and
reports for a regular state of play.

*Notifications > Rapports*: name, language (French or English), schedule (frequency, weekday or day of the month, time and IANA
time zone), where to send it, content, scope.

- **Where**: e-mail addresses (typed in the report) plus the providers/channels to use. An e-mail provider no longer holds a
  default recipient: addresses belong to the reports and rules that use it. (An address stored by an older version stays as
  a last resort.) Other channels (ntfy, Gotify, webhook) receive the text version.
- **Content**: fleet summary; updates waiting per machine and the packages most behind; **patch history** (the last N patches,
  or the last N days); failures and missed patch days; reboots required; the next patch day of each policy; container
  images with a newer version. Optionally limited to some host groups.
- **Skip when empty**: scheduled sends can be skipped when there is nothing to report; *Send now* never is.
- **Preview** shows the report as it would be sent, formatted or plain. **Send now** sends it at once, optionally only to one address.
- **Delivery log**: every attempt, with its error. A failed scheduled send is retried after 10 and 20 minutes (three tries), and each
  failure raises the `report.failed` event ("Rapport non envoyé"), so a notification rule can warn you on another channel.
  A report created today does not send this morning's slot.

E-mail is sent as plain text plus an HTML version. The *Send test* button of an e-mail channel asks who to send to (your account address
is suggested; set it in *Mon compte*).

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
