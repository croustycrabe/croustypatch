#!/bin/bash
# Builds a throwaway CroustyPatch environment for the browser tests: a scratch database, a server serving the
# exported UI over TLS 1.3, and three mock agents (a Debian web server, an Ubuntu database in quarantine, an Arch laptop).
#
# Needs: PostgreSQL (docker compose -f docker/docker-compose.yml up -d), `cargo build --workspace`,
#        `npm run build` in frontend/, curl and jq. Everything lands in frontend/e2e/.run (git-ignored).
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"; RUN="$HERE/.run"
BIN="$ROOT/target/debug"
DB="cp_e2e"; DB_URL="${E2E_DATABASE_URL:-postgres://croustypatch:croustypatch-dev@127.0.0.1:5433/$DB}"
PORT="${E2E_PORT:-8443}"; BASE="https://localhost:$PORT"

for b in croustypatch-server croustypatch-agent; do [ -x "$BIN/$b" ] || { echo "missing $BIN/$b: run cargo build --workspace"; exit 1; }; done
[ -f "$ROOT/frontend/out/index.html" ] || { echo "missing frontend/out: run npm run build"; exit 1; }

pkill -x croustypatch-se 2>/dev/null; pkill -x croustypatch-ag 2>/dev/null; sleep 1
docker exec croustypatch-pg psql -U croustypatch -d postgres -qc "drop database if exists $DB WITH (FORCE)" -c "create database $DB" || { echo "cannot reach the dev PostgreSQL container 'croustypatch-pg'"; exit 1; }
mkdir -p "$RUN/agents" && rm -rf "$RUN/data" && mkdir -p "$RUN/data"
(DATABASE_URL="$DB_URL" CROUSTYPATCH_BOOTSTRAP_PASSWORD='bootstrap-pass-123' CROUSTYPATCH_DATA_DIR="$RUN/data" CROUSTYPATCH_STATIC_DIR="$ROOT/frontend/out" CROUSTYPATCH_AGENT_DIR="$RUN/agents" \
  "$BIN/croustypatch-server" --tls-mode direct --listen "127.0.0.1:$PORT" > "$RUN/server.log" 2>&1 &)
CA="$RUN/data/tls/server.crt"; API="$BASE/api/v1"; J="$RUN/cookies.txt"; rm -f "$J"
for _ in $(seq 1 60); do curl -s --cacert "$CA" "$API/health" 2>/dev/null | grep -q ok && break; sleep 1; done
api() { local m=$1 p=$2; shift 2; curl -s --cacert "$CA" -b "$J" -c "$J" -X "$m" "$API$p" -H 'content-type: application/json' ${1:+-d "$1"}; }

curl -s -c "$J" -o /dev/null --cacert "$CA" -X POST "$API/auth/login" -H 'content-type: application/json' -d '{"username":"admin","password":"bootstrap-pass-123"}'
curl -s -b "$J" -c "$J" -o /dev/null --cacert "$CA" -X POST "$API/auth/password" -H 'content-type: application/json' -d '{"current_password":"bootstrap-pass-123","new_password":"e2e-admin-password"}'
curl -s -c "$J" -o /dev/null --cacert "$CA" -X POST "$API/auth/login" -H 'content-type: application/json' -d '{"username":"admin","password":"e2e-admin-password"}'

G1=$(api POST /groups '{"name":"prod-servers","description":"production"}' | jq -r .id); G2=$(api POST /groups '{"name":"workstations"}' | jq -r .id)
api POST /policies "{\"name\":\"default\",\"priority\":10,\"group_ids\":[\"$G1\"],\"definition\":{\"security_only\":true,\"default_action\":\"REQUIRE_APPROVAL\"}}" >/dev/null
api PUT /vulnerabilities '[{"cve_id":"CVE-2026-0001","cvss":9.8,"summary":"openssl remote code execution","exploited":true}]' >/dev/null

enrol() { # enrol NAME GROUP_JSON MOCK_JSON
  echo "$3" > "$RUN/$1.mock.json"
  printf 'package_manager = "mock"\nmock_file = "%s"\nstate_dir = "%s"\nheartbeat_secs = 10\ninventory_interval_secs = 3600\ndry_run = true\n' "$RUN/$1.mock.json" "$RUN/$1-state" > "$RUN/$1.toml"
  local tok; tok=$(api POST /enrollment-tokens "{\"label\":\"$1\",\"group_id\":$2,\"max_uses\":1}" | jq -r .token)
  CROUSTYPATCH_ENROLL_TOKEN=$tok "$BIN/croustypatch-agent" --config "$RUN/$1.toml" enroll --server "$BASE" --ca-file "$CA" --force >/dev/null 2>&1
  (RUST_LOG=warn "$BIN/croustypatch-agent" --config "$RUN/$1.toml" run > "$RUN/$1.log" 2>&1 &)
}
enrol web-01 "\"$G1\"" '{"installed":[{"name":"libssl3","version":"3.0.1","candidate":"3.0.2","is_security":true,"cves":["CVE-2026-0001"]},{"name":"nginx","version":"1.24","candidate":"1.25"},{"name":"bash","version":"5.2"}]}'
enrol db-01 "\"$G1\"" '{"installed":[{"name":"postgresql-16","version":"16.1","candidate":"16.2"},{"name":"libssl3","version":"3.0.1","candidate":"3.0.2","is_security":true,"cves":["CVE-2026-0001"]},{"name":"bash","version":"5.2"}]}'
enrol dev-laptop "\"$G2\"" '{"installed":[{"name":"firefox","version":"131","candidate":"132"},{"name":"bash","version":"5.2"}]}'
sleep 9
# agents report their own hostname; give the demo hosts readable names and varied systems
P() { docker exec croustypatch-pg psql -U croustypatch -d "$DB" -qc "$1"; }
P "update hosts set hostname='web-01', distro='Debian GNU/Linux' where id=(select id from hosts order by enrolled_at limit 1 offset 0)"
P "update hosts set hostname='db-01', distro='Ubuntu', criticality='high', status='quarantined' where id=(select id from hosts order by enrolled_at limit 1 offset 1)"
P "update hosts set hostname='dev-laptop', distro='Arch Linux', environment='dev' where id=(select id from hosts order by enrolled_at limit 1 offset 2)"

# the policy raises one approval for web-01 once the scheduler has planned it
for _ in $(seq 1 40); do [ "$(api GET /tasks | jq length)" -gt 0 ] && break; sleep 4; done

# a user who may not create enrolment tokens
ROLE=$(api GET /roles | jq -r '.[]|select(.name=="operator")|.id')
api POST /users "{\"username\":\"op1\",\"password\":\"temporary-pass-12\",\"role_id\":\"$ROLE\"}" >/dev/null
curl -s -o /dev/null -c "$RUN/op.txt" --cacert "$CA" -X POST "$API/auth/login" -H 'content-type: application/json' -d '{"username":"op1","password":"temporary-pass-12"}'
curl -s -o /dev/null -b "$RUN/op.txt" --cacert "$CA" -X POST "$API/auth/password" -H 'content-type: application/json' -d '{"current_password":"temporary-pass-12","new_password":"operator-new-password-1"}'
echo "demo environment ready: $BASE  (admin / e2e-admin-password, operator / operator-new-password-1)"
echo "hosts: $(api GET /hosts | jq length), tasks: $(api GET /tasks | jq length)"
