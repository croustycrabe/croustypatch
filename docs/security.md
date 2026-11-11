# Security model

## Channels

- **TLS 1.3 only** (rustls) for the API, the web UI and the agent tunnel. Behind a reverse proxy, the proxy
  must enforce it. Agents use TLS 1.3 only and trust either the public roots or the one pinned certificate.
- **Agent identity**: each agent generates its own RSA-2048 key at enrolment; the server stores only the
  public key. Every request carries an RS256 JWT (≤ 10 minutes, unique `jti`, replays refused).
- **Commands are signed by the server** (RSA-3072, PKCS#1 v1.5 / SHA-256) and bound to one agent, a validity
  window of 10 minutes and a nonce. The agent verifies all of it, refuses replays, and never runs a task id
  twice.
- **Enrolment tokens** are stored hashed, shown once, limited in uses and lifetime, and revocable.
- **Installer and agent downloads** (`/api/v1/agent/install.sh`, `/agent/download/*`) are public on purpose, since a
  new host has no credentials yet, and contain no secret: the token is supplied by the operator on the host. The
  script refuses to install a binary whose SHA-256 differs from the server's, and is only ever generated for a plain
  host name (never from a hostile `Host` header). Piping a script into a root shell trusts whoever serves it: the TLS
  certificate is what authenticates the server, so use the HTTPS address, and read the script first if in doubt.
  The checksum is served by the same server, so it detects a corrupted download, not a compromised server.

## Agent confinement

The agent runs as root but executes only fixed programs from system directories, without a shell, with a
wiped environment, a timeout and arguments passed one by one. Package names are validated against a strict
character set before use. Health probes accept only `tcp`, `http(s)` and service-status checks.

## Server

- Passwords: Argon2id; lockout after 5 failures for 15 minutes; a password change is forced on first login.
- Sessions: HS256 JWT in an `HttpOnly`, `SameSite=Strict` cookie (`Secure` over HTTPS); state-changing
  requests from a browser must come from the same origin (also correct over HTTP/2, where there is no `Host` header).
- RBAC: `module:action` permissions with wildcards; a user can only create roles or assign roles whose
  permissions they hold themselves.
- Secrets in the database (AI API key) are encrypted with AES-256-GCM using the master key and wiped from
  memory after use. Settings never return a stored secret.
- **Audit log**: append-only (database triggers refuse update, delete and truncate) and hash-chained, so a
  row edited by someone who disables the triggers is detected at verification. The check proves
  tampering after the fact; it cannot prove the *latest* entries were not removed unless you also record the
  head hash elsewhere (export it periodically).

## App center self-service (0.7)

Updates started from the desktop app center need no approval, so the server limits what they can do: the request
comes from the agent's own signed identity (a host only ever acts on itself), the local user comes from the socket's
peer credentials, the app center and the host's **Updates from the app center** switch must be on, the host must not
be pending or quarantined, one patch or rollback at a time, and every name must be a pending *system* update of that
machine. Everything is audited (`portal.update_requested`, task requested by `local:<user>@<host>`). Uploaded app
icons are checked by content, size-limited and only shown through `<img>`. The agent installs OpenSCAP packages
only from the fixed lists in `modules/openscap.rs`, and custom audit profiles are validated (identifiers only
letters, digits and `_ . -`) before they reach a command line or the XML tailoring file.

## Known limits

- The server → PostgreSQL link is not TLS-protected (the client library is compiled without OpenSSL). Keep it on a private network or a unix socket.
- No per-IP rate limit on sign-in (account lockout only); put a proxy rate limit in front.
- The web UI's content-security policy allows inline scripts (required by the static export).
- Operational messages written by the server and agents (task messages, risk reasoning, audit actions) are
  in English whatever the UI language; the UI's own text is fully bilingual.
- A task in flight is lost if the agent restarts; the server marks it failed after 90 minutes of silence.
- OIDC/SSO, SMTP/SMS, SIEM export and Vault are not implemented yet.
- Verified on Linux with the mock package backend and unit-tested parsers. The apt, pacman and FreeBSD paths
  run real commands that were not exercised on those systems here; `nft` and `pf` rule generation is tested
  but the rules were not applied on a live host.
