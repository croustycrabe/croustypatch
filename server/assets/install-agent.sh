#!/bin/sh
# CroustyPatch agent installer, served by __SERVER__
#
#   curl -fsSL __SERVER__/api/v1/agent/install.sh | sudo env CROUSTYPATCH_ENROLL_TOKEN=cpt_xxxxxxxx sh
#
# Optional environment:
#   CROUSTYPATCH_MODE=pull|push   pull = persistent tunnel (default), push = periodic HTTPS
#   CROUSTYPATCH_CA_FILE=PATH     trust this CA/certificate for the server (private PKI, self-signed)
#   CROUSTYPATCH_FORCE=1          enrol again even if this host is already enrolled
#   CROUSTYPATCH_DRY_RUN=1        install an agent that only logs what it would do
#
# Steps: download the agent for this platform, verify its SHA-256, install it, enrol with the token, and
# register a service (systemd on Linux, rc.d on FreeBSD). Running it again upgrades the binary and keeps the
# existing enrolment.
set -eu

SERVER="__SERVER__"
BASE="$SERVER/api/v1/agent"

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run as root, for example: curl -fsSL $BASE/install.sh | sudo env CROUSTYPATCH_ENROLL_TOKEN=... sh"
TOKEN="${CROUSTYPATCH_ENROLL_TOKEN:-}"
MODE="${CROUSTYPATCH_MODE:-pull}"
case "$MODE" in pull|push) ;; *) die "CROUSTYPATCH_MODE must be 'pull' or 'push'" ;; esac

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS-$ARCH" in
  Linux-x86_64|Linux-amd64)   PLAT=linux-x86_64 ;;
  Linux-aarch64|Linux-arm64)  PLAT=linux-aarch64 ;;
  FreeBSD-amd64|FreeBSD-x86_64) PLAT=freebsd-amd64 ;;
  *) die "unsupported platform: $OS $ARCH" ;;
esac

dl() { # dl URL DEST
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL ${CROUSTYPATCH_CA_FILE:+--cacert "$CROUSTYPATCH_CA_FILE"} -o "$2" "$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q ${CROUSTYPATCH_CA_FILE:+--ca-certificate="$CROUSTYPATCH_CA_FILE"} -O "$2" "$1"
  elif command -v fetch >/dev/null 2>&1; then
    fetch -q ${CROUSTYPATCH_CA_FILE:+--ca-cert="$CROUSTYPATCH_CA_FILE"} -o "$2" "$1"
  else
    die "need curl, wget or fetch"
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v sha256 >/dev/null 2>&1; then sha256 -q "$1"
  else die "need sha256sum or sha256 to verify the download"; fi
}

if [ "$OS" = FreeBSD ]; then
  CONF=/usr/local/etc/croustypatch/agent.toml
  STATE=/var/db/croustypatch
else
  CONF=/etc/croustypatch/agent.toml
  STATE=/var/lib/croustypatch
fi
# An enrolled host upgrades without a token; a new (or forced) enrolment needs one. Checked before anything is touched.
if [ "${CROUSTYPATCH_FORCE:-0}" = 1 ] || [ ! -f "$STATE/identity.json" ]; then
  [ -n "$TOKEN" ] || die "CROUSTYPATCH_ENROLL_TOKEN is not set (create one in the web UI: Hosts > Add agent)"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

say "downloading the agent for $PLAT"
dl "$BASE/download/$PLAT" "$TMP/agent" || die "could not download the agent for $PLAT; this server may not provide that platform"
dl "$BASE/download/$PLAT.sha256" "$TMP/agent.sha256" || die "could not download the checksum"
WANT="$(cut -d' ' -f1 "$TMP/agent.sha256")"
GOT="$(sha256_of "$TMP/agent")"
[ -n "$WANT" ] && [ "$WANT" = "$GOT" ] || die "checksum mismatch (expected $WANT, got $GOT): refusing to install"
say "checksum verified"

BIN=/usr/local/bin/croustypatch-agent

mkdir -p "$(dirname "$BIN")" "$(dirname "$CONF")" "$STATE"
chmod 700 "$STATE" "$(dirname "$CONF")"
install -m 0755 "$TMP/agent" "$BIN.new"
mv -f "$BIN.new" "$BIN"
say "installed $BIN"
# The terminal app center, for local users without root: `croustypatch-portal`.
ln -sf "$BIN" "$(dirname "$BIN")/croustypatch-portal"

FORCE_ARG=""
[ "${CROUSTYPATCH_FORCE:-0}" = 1 ] && FORCE_ARG="--force"
if [ -f "$STATE/identity.json" ] && [ -z "$FORCE_ARG" ]; then
  say "this host is already enrolled: only the binary was upgraded (CROUSTYPATCH_FORCE=1 enrols again)"
else
  say "enrolling with $SERVER (mode: $MODE)"
  # shellcheck disable=SC2086
  CROUSTYPATCH_ENROLL_TOKEN="$TOKEN" "$BIN" --config "$CONF" --state-dir "$STATE" enroll \
    --server "$SERVER" --mode "$MODE" ${CROUSTYPATCH_CA_FILE:+--ca-file "$CROUSTYPATCH_CA_FILE"} $FORCE_ARG
  if [ "${CROUSTYPATCH_DRY_RUN:-0}" = 1 ]; then
    grep -q '^dry_run' "$CONF" && sed -i.bak 's/^dry_run.*/dry_run = true/' "$CONF" && rm -f "$CONF.bak" || echo 'dry_run = true' >> "$CONF"
    say "dry-run enabled: the agent will not change this system"
  fi
fi

# On apt systems the agent saves a restore point before patching. The old version of an upgraded package has usually
# left the repositories by then, so dpkg-repack (which rebuilds a package from the installed files) is what makes
# rollback possible. Best effort: a missing package must not stop the installation.
if [ "$OS" = Linux ] && command -v apt-get >/dev/null 2>&1 && ! command -v dpkg-repack >/dev/null 2>&1; then
  say "installing dpkg-repack (restore points for packages whose old version left the repositories)"
  # Inside some containers apt cannot drop to its sandbox user; retry without the sandbox (we are root anyway).
  apt_run() { DEBIAN_FRONTEND=noninteractive apt-get "$@" || DEBIAN_FRONTEND=noninteractive apt-get -o APT::Sandbox::User=root "$@"; }
  apt_run install -y -qq dpkg-repack >/dev/null 2>&1 \
    || { apt_run update -qq >/dev/null 2>&1; apt_run install -y -qq dpkg-repack >/dev/null 2>&1; } \
    || say "warning: could not install dpkg-repack; patches that cannot be restored from the repositories will be refused (apt install dpkg-repack)"
fi

if [ "$OS" = Linux ] && [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
  cat > /etc/systemd/system/croustypatch-agent.service <<'UNIT'
[Unit]
Description=CroustyPatch agent
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/croustypatch-agent run
Restart=always
RestartSec=5
User=root
# No systemd sandboxing on purpose: the agent runs the package manager, whose maintainer scripts install kernel
# modules, rebuild the initramfs, switch users and restart services. ProtectKernelModules, ProtectSystem, PrivateTmp,
# NoNewPrivileges and the like make those scripts fail half-way (and the rollback with them). The agent confines its
# own commands instead (no shell, fixed programs, cleared environment) and refuses to patch if it finds itself
# sandboxed (see agent/src/preflight.rs).

[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload
  systemctl enable croustypatch-agent >/dev/null 2>&1 || true
  systemctl restart croustypatch-agent
  say "service started: systemctl status croustypatch-agent"
elif [ "$OS" = FreeBSD ]; then
  mkdir -p /usr/local/etc/rc.d
  cat > /usr/local/etc/rc.d/croustypatch_agent <<'RC'
#!/bin/sh
# PROVIDE: croustypatch_agent
# REQUIRE: NETWORKING DAEMON
# KEYWORD: shutdown
. /etc/rc.subr
name="croustypatch_agent"
rcvar="croustypatch_agent_enable"
pidfile="/var/run/${name}.pid"
command="/usr/sbin/daemon"
command_args="-r -P ${pidfile} -u root -t croustypatch-agent /usr/local/bin/croustypatch-agent --config /usr/local/etc/croustypatch/agent.toml --state-dir /var/db/croustypatch run"
load_rc_config $name
: ${croustypatch_agent_enable:="NO"}
run_rc_command "$1"
RC
  chmod 555 /usr/local/etc/rc.d/croustypatch_agent
  sysrc croustypatch_agent_enable=YES >/dev/null
  service croustypatch_agent restart >/dev/null 2>&1 || service croustypatch_agent start
  say "service started: service croustypatch_agent status"
  say "to allow network isolation add this line to /etc/pf.conf and enable pf:  anchor \"croustypatch_quarantine\""
else
  say "no supported service manager found; start the agent with: $BIN --config $CONF --state-dir $STATE run"
fi

say "done: the host should appear in the web UI within a minute"
