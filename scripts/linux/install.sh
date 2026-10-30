#!/usr/bin/env bash
# Install (or update) Outpost on a Linux server, as your user, from the newest release.
#
#   curl -fsSL https://github.com/danite1252-ctrl/OutpostAI/releases/latest/download/install.sh | bash
#
# Puts the program in ~/.local/lib/outpost, runs it as a systemd user service ("outpost",
# with --server: network access on), and shows the setup code for creating the owner from
# the desktop app. Needs: x86_64 Linux with systemd, curl, python3, sha256sum.
set -euo pipefail

REPO="danite1252-ctrl/OutpostAI"
BASE="https://github.com/$REPO/releases/latest/download"
BIN_DIR="$HOME/.local/lib/outpost"
UNIT_DIR="$HOME/.config/systemd/user"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

if [ "$(uname -m)" != "x86_64" ]; then
  echo "Outpost's server build is for x86_64 Linux; this is $(uname -m)." >&2
  exit 1
fi

echo "Finding the newest Outpost..."
curl -fsSL "$BASE/latest.json" -o "$TMP/latest.json"
read -r VERSION URL SHA < <(python3 - "$TMP/latest.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
p = m.get("platforms", {}).get("linux-x86_64") or {}
print(m.get("version", ""), p.get("url", ""), p.get("sha256", ""))
PY
)
if [ -z "$URL" ]; then
  echo "The newest release has no Linux server build yet." >&2
  exit 1
fi

echo "Downloading Outpost $VERSION..."
curl -fL --progress-bar "$URL" -o "$TMP/outpost-gateway"
if [ -n "$SHA" ]; then
  echo "$SHA  $TMP/outpost-gateway" | sha256sum -c --quiet - || { echo "The download is damaged; nothing was installed." >&2; exit 1; }
fi

mkdir -p "$BIN_DIR" "$UNIT_DIR"
install -m 0755 "$TMP/outpost-gateway" "$BIN_DIR/outpost-gateway"
cat >"$UNIT_DIR/outpost.service" <<UNIT
# Outpost on a Linux server, as a systemd user service (from install.sh).
[Unit]
Description=Outpost (local AI server)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=%h/.local/lib/outpost/outpost-gateway --server
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=outpost_gateway=info,outpost_core=info,outpost_engines=info

[Install]
WantedBy=default.target
UNIT

systemctl --user daemon-reload
systemctl --user enable outpost >/dev/null
systemctl --user restart outpost
echo "Outpost $VERSION is running."

if ! loginctl show-user "$USER" -p Linger 2>/dev/null | grep -q "yes"; then
  echo
  echo "To keep Outpost running when you're logged out (and start it at boot), run once:"
  echo "  sudo loginctl enable-linger $USER"
fi

sleep 4
echo
"$BIN_DIR/outpost-gateway" --setup-code || true
echo
echo "If your server uses ufw, allow paired devices from your home network, e.g.:"
echo "  sudo ufw allow from 192.168.1.0/24 to any port 7421 proto tcp"
