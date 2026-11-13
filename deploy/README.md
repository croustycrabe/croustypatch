# Running CroustyPatch with Docker Compose

This directory runs the CroustyPatch server and its PostgreSQL 17 database on one Docker host. You need Docker with
the Compose plugin (v2) and `openssl`.

## Quickstart

```sh
cd deploy
cp .env.example .env && chmod 600 .env
```

Edit `.env` and fill in the two secrets (the comments in the file give the command for each):

```sh
openssl rand -hex 24      # CROUSTYPATCH_DB_PASSWORD
openssl rand -base64 32   # CROUSTYPATCH_MASTER_KEY
```

Pin the version you want in `CROUSTYPATCH_VERSION` (for example `1.0.0`; `latest` follows the newest release), then:

```sh
docker compose up -d
docker compose ps                      # both services should become "healthy"
docker compose logs server | head -30  # the first-login password, shown once
```

The server listens on `https://<host>:8443` with a self-signed TLS 1.3 certificate. Put the name you will use in
`CROUSTYPATCH_TLS_SAN` (for example `patch.example.org,10.0.0.5`) before the first start, since the certificate is
created then. For a certificate trusted by your browsers, use a reverse proxy (see below).

**Back up the master key now** (copy `CROUSTYPATCH_MASTER_KEY` to a password manager). It encrypts sensitive values and
derives the session key; the database backup does not contain it, and without it encrypted values cannot be recovered.

## First login

1. Open `https://<host>:8443` and sign in as `admin` with the password from the log (or the one you set in
   `CROUSTYPATCH_BOOTSTRAP_PASSWORD`). You must change it straight away.
2. Create your own administrator accounts (**Administration**), then disable the fallback account:

   ```sh
   docker compose exec server croustypatch-server admin disable --super-admin-fallback
   ```

   The command refuses to run if no other enabled account could manage users, so you cannot lock yourself out.
   Other commands: `admin enable --super-admin-fallback`, `admin reset-password <user>` (new password in the
   `CROUSTYPATCH_NEW_PASSWORD` variable), `migrate`.

## Enrol the first agent

In the web UI choose **Add agent**: the wizard shows a one-line install command for Linux with a one-time token that
expires. Run it as root on the machine to manage. The agent installs itself as a service and the host appears in the
list a few seconds later. With a self-signed certificate the wizard's command also tells the agent which certificate to
trust.

The server hands its own agent binary to new hosts. If you need other builds, drop `croustypatch-agent-<platform>`
files into the `serverdata` volume under `agents/` (`/var/lib/croustypatch/agents` in the container).

## Reverse proxy and TLS

For a public name and a trusted certificate, run the server behind a reverse proxy (Caddy, Traefik, nginx, Zoraxy...)
that terminates TLS, and set in `.env`:

```
CROUSTYPATCH_TLS_MODE=proxy
CROUSTYPATCH_BIND=127.0.0.1                  # publish the plain-HTTP port to the proxy only
CROUSTYPATCH_PORT=8443
CROUSTYPATCH_TRUSTED_PROXIES=172.18.0.1      # the proxy's address as the container sees it
```

The proxy must forward to `http://127.0.0.1:8443`, keep WebSocket upgrades working (agents connect over a WebSocket)
and set `X-Forwarded-For` and `X-Forwarded-Proto`. The server honours those headers only from the addresses in
`CROUSTYPATCH_TRUSTED_PROXIES`; with the port published by Docker the proxy usually appears as the bridge gateway (look
in the server log if client addresses are wrong). A proxy that runs in this same compose project can be reached as
`http://server:8443` on the `internal` network, and then needs no published port.

### Your own certificate (direct mode)

Add a `compose.override.yml` next to `docker-compose.yml`:

```yaml
services:
  server:
    environment:
      CROUSTYPATCH_TLS_CERT: /certs/fullchain.pem
      CROUSTYPATCH_TLS_KEY: /certs/privkey.pem
    volumes:
      - ./certs:/certs:ro
```

and run `docker compose -f docker-compose.yml -f compose.override.yml up -d`.

## Upgrade

1. Read the release notes, and take a backup (below).
2. Change `CROUSTYPATCH_VERSION` in `.env` to the new version.
3. `docker compose pull && docker compose up -d`

Database migrations apply when the server starts. To go back, restore the backup taken before the upgrade and set the
previous version again (newer migrations are not always reversible).

## Backup and restore

The data is in two named volumes: `pgdata` (the database) and `serverdata` (generated certificate, served agent
binaries). Back up the database with `pg_dump`, and the master key separately:

```sh
docker compose exec -T database pg_dump -U croustypatch -d croustypatch | gzip > croustypatch-$(date +%Y%m%d-%H%M).sql.gz
```

Restore into an empty database (a fresh install, or after `docker compose down -v` on a test copy):

```sh
docker compose up -d database
gunzip -c croustypatch-YYYYMMDD-HHMM.sql.gz | docker compose exec -T database psql -U croustypatch -d croustypatch
docker compose up -d
```

Use the **same master key** as when the backup was taken. The audit log's integrity can be checked from the UI
(**Audit** → verify the chain) after a restore.

## Notes

- Logs: `docker compose logs -f server`. The health endpoint is `/api/v1/health`.
- The server container runs as an unprivileged user with a read-only root file system and no capabilities.
- Stop: `docker compose down` keeps the data; `docker compose down -v` deletes the volumes, and with them the database.
