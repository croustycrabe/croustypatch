# Testing

Tests live in each crate's `tests/` folder (Rust convention), never inline in `src/`. The server and agent are
a thin `main.rs` over a library crate precisely so that tests can import the real code.

```sh
docker compose -f docker/docker-compose.yml up -d   # PostgreSQL for the database-backed tests
cargo test --workspace
```

| Crate | Files | What they cover |
|---|---|---|
| `common` | `crypto`, `jwt`, `model`, `protocol` | RSA/HMAC primitives, JWT algorithm confusion, signed command verification |
| `server` | `engine_*`, `crypto`, `auth`, `audit`, `planner`, `scheduler` | Rule language, risk and decision logic, maintenance windows, permission patterns, CSRF host detection |
| `server` | `db_planner`, `db_audit_and_auth` | **Against real PostgreSQL**: planner, waivers, audit chain and tamper detection, concurrent appends, bootstrap and lock-out guard, command dispatch |
| `server` | `http_api` | **Real HTTP stack on a real database**: login, lockout, forced password change, CSRF over HTTP/2-style requests, RBAC and privilege escalation, enrolment, agent JWT replay |
| `agent` | `pkg_*`, `exec`, `isolate`, `health`, … | Package-manager output parsers (apt, pacman, pkg), sandboxed execution, firewall rule generation |
| `server` | `http_fleet` | **Add agent** distribution (installer targets the right address and rejects hostile `Host` headers; binaries listed, served, checksummed, no path escape), fleet-wide package aggregation and dashboard aggregates |
| `agent` | `pipeline`, `link` | The real task runner end to end with a simulated service: patch, soak, automatic rollback, double failure (quarantine and evidence), replay, manual rollback; command verification on the wire |

## Database-backed tests

They need a PostgreSQL server where the role may create databases. The default is the development one from
`docker/docker-compose.yml`; override with `TEST_DATABASE_URL=postgres://user:pass@host:port/postgres`.
Each test creates its own `cp_test_<random>` database and drops it afterwards, so they are safe to run in
parallel and leave nothing behind. If PostgreSQL is unreachable they **fail** with instructions rather than skip.

## Regression tests

Two bugs were only visible end to end, and each has a test that fails without the fix:

- `db_planner::policy_linked_to_two_groups_of_the_host_still_plans`: `SELECT DISTINCT … ORDER BY` rejected by PostgreSQL.
- `http_api::csrf_accepts_same_origin_over_http2_style_requests_and_refuses_foreign_origins`: no `Host` header over HTTP/2.
- `http_api::enrolment_tokens_are_single_use_and_keys_are_validated`: a rejected enrolment must stay in the audit log (it used to be rolled back with the failed transaction).

## Not covered

The apt, pacman and FreeBSD `pkg` command paths are tested through their output parsers and the mock backend,
not on those systems; `nft` and `pf` rules are generated and checked but not applied. The web UI is covered by the browser suite
below, which is not part of `cargo test`.

## Browser tests

`frontend/e2e/` drives Chromium (Playwright) against a real server, database and agents, including the Add-agent flow
end to end (a real agent enrols with the wizard's token and the wizard must notice it). See `frontend/e2e/README.md`.

```sh
cd frontend && npx playwright install chromium && npm run build && npm run e2e
```

The generated installer script was also run for real in a clean Debian container (strict POSIX `sh`): missing token
refused, checksum verified and a forced mismatch refused, install and enrolment, idempotent re-run, agent connected.
The systemd unit it writes was validated with `systemd-analyze`; the FreeBSD branch and a real systemd restart were not exercised.
