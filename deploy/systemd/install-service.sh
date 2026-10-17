#!/usr/bin/env bash
# One-shot installer for dns-leak-monitor: builds the binary, creates the
# service user and data dir, installs config + unit, and starts the service.
#
#   ./deploy/systemd/install-service.sh                 # prompts for the capture interface
#   IFACE=eth0 ./deploy/systemd/install-service.sh      # non-interactive
#
# Run it as your normal user, NOT with sudo: the cargo build runs unprivileged
# and the script escalates on its own for the install step only.
#
# Re-running is safe: an existing config is kept, never overwritten.
set -euo pipefail

BIN_DEST=/usr/local/bin/dns-leak-monitor
CONF_DIR=/etc/dns-leak-monitor
CONF=$CONF_DIR/config.toml
DATA_DIR=/var/lib/dns-leak-monitor
UNIT=/etc/systemd/system/dns-leak-monitor.service
SVC_USER=dnswatch
PORT=28107

DEPLOY_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
SRC_DIR=$(cd -- "$DEPLOY_DIR/../.." && pwd)

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m warn:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

command -v systemctl >/dev/null || die "systemd not found; this installer targets systemd hosts"

# The build runs as the invoking user and the install as root. Phase 2 is this
# same script re-executed under sudo with the already-built binary handed over,
# so cargo is never looked up in root's PATH (rustup installs into ~/.cargo,
# which root cannot see).
PHASE=${DLM_PHASE:-build}

# --- capture interface ------------------------------------------------------
# Wrong interface = silently captures nothing, so make the default explicit.
if [[ -z ${IFACE:-} ]] && [[ -f $CONF ]]; then
  IFACE=$(sed -n 's/^[[:space:]]*interface[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$CONF" | head -1)
  [[ -n $IFACE ]] && say "reusing interface from $CONF: $IFACE"
fi
if [[ -z ${IFACE:-} ]]; then
  DEFAULT_IFACE=$(ip route show default 2>/dev/null | awk '/default/ {print $5; exit}' || true)
  echo "Available interfaces: $(ls /sys/class/net | tr '\n' ' ')"
  read -r -p "Capture interface [${DEFAULT_IFACE:-none}]: " IFACE || true
  IFACE=${IFACE:-$DEFAULT_IFACE}
fi
[[ -n ${IFACE:-} ]] || die "no capture interface given (set IFACE=... or answer the prompt)"
[[ -e /sys/class/net/$IFACE ]] || warn "interface '$IFACE' does not exist right now — continuing anyway"

# --- phase 1: build as the invoking user, then escalate ---------------------
if [[ $PHASE == build ]]; then
  say "building release binary"
  command -v cargo >/dev/null || die "cargo not found; install Rust (https://rustup.rs)"
  cargo build --release --manifest-path "$SRC_DIR/Cargo.toml"
  BUILT=$SRC_DIR/target/release/dns-leak-monitor
  [[ -x $BUILT ]] || die "build reported success but $BUILT is missing"

  if [[ $(id -u) -eq 0 ]]; then
    DLM_PHASE=install DLM_BINARY=$BUILT IFACE=$IFACE bash "$0" "$@"
  else
    command -v sudo >/dev/null || die "not root and sudo not found; re-run as root"
    say "build done — root privileges required for the install step"
    sudo DLM_PHASE=install DLM_BINARY="$BUILT" IFACE="$IFACE" bash "$0" "$@"
  fi
  exit $?
fi

# --- phase 2 (root): install ------------------------------------------------
[[ $(id -u) -eq 0 ]] || die "install phase must run as root"
[[ -x ${DLM_BINARY:-} ]] || die "internal: DLM_BINARY not handed over from the build phase"

say "installing binary to $BIN_DEST"
install -m 0755 "$DLM_BINARY" "$BIN_DEST"

# --- user, dirs, config -----------------------------------------------------
if ! id -u "$SVC_USER" >/dev/null 2>&1; then
  say "creating system user $SVC_USER"
  useradd --system --no-create-home --shell /usr/sbin/nologin "$SVC_USER"
fi

install -d -m 0750 -o "$SVC_USER" -g "$SVC_USER" "$DATA_DIR"
install -d -m 0755 "$CONF_DIR"

if [[ -f $CONF ]]; then
  say "keeping existing $CONF"
else
  say "writing $CONF"
  cat > "$CONF" <<EOF
interface      = "$IFACE"
db_path        = "$DATA_DIR/dns.db"
bind           = "127.0.0.1:$PORT"    # localhost only; reach via SSH tunnel
retention_days = 90

# Outbound queries whose qname matches ANY entry are dropped, never stored.
# Match = case-insensitive suffix: "example.com" also matches "a.example.com".
# Over-exclusion is permanent — dropped queries cannot be reviewed later.
exclude = [
  "in-addr.arpa",
  "ip6.arpa",
]
EOF
  chmod 0644 "$CONF"
fi

# --- unit -------------------------------------------------------------------
say "installing systemd unit"
install -m 0644 "$DEPLOY_DIR/dns-leak-monitor.service" "$UNIT"

systemctl daemon-reload
systemctl enable dns-leak-monitor >/dev/null
say "starting service"
systemctl restart dns-leak-monitor

sleep 1
if systemctl is-active --quiet dns-leak-monitor; then
  say "dns-leak-monitor is running on interface '$IFACE'"
  echo
  echo "  console:  http://127.0.0.1:$PORT"
  echo "  tunnel:   ssh -L $PORT:127.0.0.1:$PORT $(logname 2>/dev/null || echo user)@<this-host>"
  echo "  logs:     journalctl -u dns-leak-monitor -f"
  echo "  config:   $CONF   (edit, then: systemctl reload dns-leak-monitor)"
else
  warn "service failed to start; recent log:"
  journalctl -u dns-leak-monitor -n 20 --no-pager || true
  exit 1
fi
