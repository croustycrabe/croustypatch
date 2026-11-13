# CroustyPatch on a Docker host, behind a reverse proxy

`compose.yaml` here is the stack as deployed on `docker-admin` (`/opt/stacks/croustypatch`), where it replaced
PatchMon. TLS 1.3 is terminated by the reverse proxy (Zoraxy at 10.0.15.10), which forwards
`patchmon.lan.croustylab.org` to `docker-admin:3000`; the container speaks plain HTTP on that port and trusts
`X-Forwarded-*` only from `CROUSTYPATCH_TRUSTED_PROXIES`.

## Files on the host

| Path | Content |
|---|---|
| `/opt/stacks/croustypatch/compose.yaml` | this file |
| `/opt/stacks/croustypatch/.env` (mode 600) | database password, **master key**, trusted proxy, one-time admin password, image tag, port |
| `/opt/stacks/croustypatch/postgres/` | database files |
| docker volume `croustypatch_serverdata` | server data directory |

**Back up the master key (`CROUSTYPATCH_MASTER_KEY` in `.env`) separately from the database**, for example in the
password manager. Without it the encrypted settings cannot be read.

## First login

The `admin` account starts with the password in `.env` (`CROUSTYPATCH_BOOTSTRAP_PASSWORD`) and must change it
at first login. Afterwards delete that line from `.env`. Once real administrators exist:

    docker compose exec server croustypatch-server admin disable --super-admin-fallback

## Operate

    cd /opt/stacks/croustypatch
    docker compose ps                      # both services should be (healthy)
    docker compose logs -f server

The image `croustypatch-server:<version>` was built from the repository and loaded with
`docker save croustypatch-server:<version> | gzip | ssh host 'gunzip | docker load'`. To upgrade, load the new image,
change `CROUSTYPATCH_IMAGE` in `.env`, and `docker compose up -d` (migrations run at start).

## Agents served by "Add agent"

The image contains the Linux agent. Another build (for example aarch64) can be dropped into the persisted volume:

    docker cp croustypatch-agent-linux-aarch64 croustypatch-server-1:/var/lib/croustypatch/agents/

The wizard shows which platforms are available. The installer URL is generated from the address the browser used,
so the reverse proxy must pass `Host`/`X-Forwarded-Host` and `X-Forwarded-Proto`.

## Reverse-proxy requirements

- Forward to `http://docker-admin:3000` and pass `Host`, `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Host`.
- Allow WebSocket upgrades on `/api/v1/agent/ws` and keep idle connections for at least a few minutes
  (verified through Zoraxy: 100 s idle, no reconnection).
- Set the minimum TLS version to 1.3 on the proxy, and add `Strict-Transport-Security` there: in proxy mode the
  server cannot enforce either.

## Rolling back to PatchMon

PatchMon was stopped, not deleted. Its data, configuration and a verified database dump are still in
`/opt/stacks/patchmon` (`backup-before-croustypatch-*.sql.gz`).

    cd /opt/stacks/croustypatch && docker compose down        # keeps the data; add -v only to discard it
    cd /opt/stacks/patchmon && docker compose up -d

Note: PatchMon's server container was already failing before the migration (it could not resolve its `database`
host name), so a rollback needs that fixed first.
