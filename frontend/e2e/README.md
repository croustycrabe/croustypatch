# Browser tests

A scripted Chromium drives the real interface against a real server, database and agents: sign-in, navigation,
filters and sorting, bulk selection, tabs, the ⌘K palette, package and task panels, the policy editor, role-based
visibility, language switching, and the **Add agent** flow end to end (the wizard generates a command, a real agent
enrols with its token, and the wizard must notice the new host).

```sh
docker compose -f docker/docker-compose.yml up -d      # PostgreSQL
cargo build --workspace                                # server and agent binaries
npx playwright install chromium                        # once
npm run build && npm run e2e                           # from frontend/
```

`e2e/setup-demo.sh` creates a scratch database `cp_e2e`, starts the server on `https://localhost:8443`, enrols three
mock agents and creates an operator account; everything lands in `e2e/.run` (git-ignored). It stops any running
`croustypatch-server` and `croustypatch-agent` first. Each run needs a fresh environment: approving a task makes an
agent re-report its real hostname, so rerunning the tests without rerunning the setup will fail on host names.

Variables: `E2E_BASE` (default `https://localhost:8443`), `E2E_PORT`, `E2E_DATABASE_URL`, `AGENT_BIN`.
