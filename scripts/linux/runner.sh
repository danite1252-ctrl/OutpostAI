#!/usr/bin/env bash
# Outpost Linux test runner (for building Outpost on the server while it's being made).
#
# Claude drops a flag file in the shared folder's target/overnight-linux/; this script then
# runs ONLY the fixed jobs below and writes the output next to it. It never commits, never
# pushes, and can't run anything else.
#
#   test      copy the latest code from the share, cargo test             -> linux-result.txt
#   build     copy the latest code, build the gateway (release)           -> linux-result.txt
#   deploy    build, then install it as the "outpost" user service and restart it
#                                                                          -> linux-result.txt
#   release   build the gateway for a release (the release script on Windows asks for it)
#                                                                          -> release-out/
#   stop      stop the outpost service
#   status    machine and service status (GPU, memory, disk, service log)  -> linux-status.txt
#   logs      the outpost service's last 400 log lines                     -> linux-logs.txt
#
# Runs as your user (not root) as a systemd user service: see docs/linux-runner.md.

set -u
SHARE="${OUTPOST_SHARE:-/mnt/outpost}"
FLAGS="$SHARE/target/overnight-linux"
SRC="$HOME/outpost-src"
BIN="$HOME/.local/lib/outpost"
export CARGO_TARGET_DIR="$HOME/outpost-target"
export PATH="$HOME/.cargo/bin:$PATH"
export NO_COLOR=1
# Gentle on a small server: two compile jobs at a time and no debug info, which roughly
# halves the memory a build needs (a full-speed build can exhaust a 16 GB Proxmox host).
export CARGO_BUILD_JOBS="${OUTPOST_BUILD_JOBS:-2}"
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_RELEASE_DEBUG=0

mkdir -p "$FLAGS" "$BIN"
log() { echo "$*" >>"$OUT"; }
step() {
  local name="$1"; shift
  log "--- $name ---"
  "$@" >>"$OUT" 2>&1
  local code=$?
  log "$name exit: $code"
  return $code
}
sync_src() {
  # Local copy, so builds don't run over the network share (and build output stays local).
  rsync -a --delete \
    --exclude target --exclude node_modules --exclude dist --exclude .git \
    --exclude apps/mobile/android --exclude apps/mobile/.expo \
    "$SHARE/" "$SRC/"
}
build() {
  step "copy code" sync_src &&
    (cd "$SRC" && step "cargo build --release" cargo build --release -p outpost-gateway)
}

echo "Outpost Linux runner is waiting for jobs in $FLAGS"
while true; do
  for job in test build deploy release stop status logs; do
    [ -e "$FLAGS/$job" ] || continue
    rm -f "$FLAGS/$job"
    case "$job" in
      status) OUT="$FLAGS/linux-status.txt" ;;
      logs) OUT="$FLAGS/linux-logs.txt" ;;
      *) OUT="$FLAGS/linux-result.txt" ;;
    esac
    echo "=== $job started $(date '+%F %T') ===" >"$OUT"
    ok=0
    case "$job" in
      test)
        step "copy code" sync_src && (cd "$SRC" && step "cargo test" cargo test) || ok=1
        ;;
      build)
        build || ok=1
        ;;
      deploy)
        if build; then
          systemctl --user stop outpost 2>/dev/null
          cp "$CARGO_TARGET_DIR/release/outpost-gateway" "$BIN/outpost-gateway"
          mkdir -p "$HOME/.config/systemd/user"
          cp "$SRC/scripts/linux/outpost.service" "$HOME/.config/systemd/user/outpost.service"
          systemctl --user daemon-reload
          step "enable and start" systemctl --user enable --now outpost || ok=1
          sleep 5
          step "service status" systemctl --user --no-pager status outpost
          step "health" curl -s --max-time 5 http://127.0.0.1:7420/api/health
        else
          ok=1
        fi
        ;;
      release)
        # The Linux server program for a GitHub release, beside its version, for
        # scripts/release/release.ps1 to sign and attach.
        rm -rf "$FLAGS/release-out"
        if build; then
          mkdir -p "$FLAGS/release-out"
          cp "$CARGO_TARGET_DIR/release/outpost-gateway" "$FLAGS/release-out/outpost-gateway-linux-x86_64" &&
            "$CARGO_TARGET_DIR/release/outpost-gateway" --version | awk '{print $2}' >"$FLAGS/release-out/version.txt" ||
            ok=1
          step "built" ls -l "$FLAGS/release-out"
        else
          ok=1
        fi
        ;;
      stop)
        step "stop" systemctl --user stop outpost || ok=1
        ;;
      status)
        step "host" hostnamectl
        step "gpu" nvidia-smi
        step "memory" free -h
        step "disk" df -h / "$HOME"
        step "service" systemctl --user --no-pager status outpost
        step "health" curl -s --max-time 5 http://127.0.0.1:7420/api/health
        ;;
      logs)
        step "outpost log" journalctl --user -u outpost -n 400 --no-pager
        # The engines' own logs (Ollama, ComfyUI, the voice service).
        for f in "$HOME"/.local/share/Outpost/engines/*/logs/*.log; do
          [ -f "$f" ] && step "$(basename "$f")" tail -n 120 "$f"
        done
        ;;
    esac
    echo "=== $job finished $(date '+%F %T') (ok: $([ $ok -eq 0 ] && echo true || echo false)) ===" >>"$OUT"
  done
  sleep 5
done
