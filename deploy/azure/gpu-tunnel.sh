#!/usr/bin/env bash
# Lends this machine's GPU to the Azure stack. A chisel client dials the VM over HTTPS (port 443,
# nothing is opened on this machine) and publishes the local Ollama as gpu-tunnel.internal:11436 inside the
# stack, where the memory engine reads it as EXTRACTION_OLLAMA_URL.
#   gpu-tunnel.sh install   download the pinned chisel client into ~/.local/opt/chisel (checksum verified)
#   gpu-tunnel.sh up        start the tunnel in the background
#   gpu-tunnel.sh down      stop it
#   gpu-tunnel.sh status    say whether it is running and whether the VM sees the model server
# Needs the local Ollama to serve the model first (scripts/fetch-slm.sh with OLLAMA_URL set to it).
#   GPU_OLLAMA_URL   the local Ollama to lend (default http://127.0.0.1:11436)
#   EXTRACTION_MODEL the model that must be installed there (default mem-extractor)
# If this machine is off or asleep the memory worker cannot reach a model: extraction jobs fail
# with a clear error and are retried, nothing falls back to another model.
set -euo pipefail

STATE=${SDP_CLOUD_STATE:-$HOME/.sdp-cloud}
OLLAMA=${GPU_OLLAMA_URL:-http://127.0.0.1:11436}
MODEL=${EXTRACTION_MODEL:-mem-extractor}
VERSION=1.12.0
SHA256=f3f180f1d93aa72cce4e6386f98cc06569a0146fbd65eb4423cf83e6434bcfe6
DEST=$HOME/.local/opt/chisel
BIN=${CHISEL_BIN:-$DEST/chisel-bin}
PIDFILE=$STATE/gpu-tunnel.pid
LOG=$STATE/gpu-tunnel.log
# shellcheck disable=SC1091
. "$STATE/azure.env"
FQDN=$DNS.$LOC.cloudapp.azure.com
secret() { grep -m1 '^GPU_TUNNEL_SECRET=' "$STATE/secrets.env" | cut -d= -f2-; }
running() { [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2> /dev/null; }

case "${1:-}" in
  install)
    mkdir -p "$DEST"
    curl -fsSL -o "$DEST/chisel.gz" "https://github.com/jpillora/chisel/releases/download/v$VERSION/chisel_${VERSION}_linux_amd64.gz"
    echo "$SHA256  $DEST/chisel.gz" | sha256sum -c - > /dev/null || { echo "checksum mismatch for chisel $VERSION" >&2; exit 1; }
    gunzip -f "$DEST/chisel.gz" && mv "$DEST/chisel" "$BIN" && chmod +x "$BIN"
    echo "chisel $("$BIN" --version) installed at $BIN" ;;
  up)
    [ -x "$BIN" ] || { echo "chisel is not installed: run $0 install" >&2; exit 1; }
    running && { echo "already running (pid $(cat "$PIDFILE"))"; exit 0; }
    curl -fsS --max-time 5 "$OLLAMA/api/show" -d "{\"model\":\"$MODEL\"}" > /dev/null \
      || { echo "$MODEL is not served at $OLLAMA. Start Ollama there and run scripts/fetch-slm.sh with OLLAMA_URL=$OLLAMA" >&2; exit 1; }
    [ -n "$(secret)" ] || { echo "no GPU_TUNNEL_SECRET in $STATE/secrets.env: run deploy/azure/sync-config.sh first" >&2; exit 1; }
    : > "$LOG"; chmod 600 "$LOG"
    AUTH="gpu:$(secret)" nohup "$BIN" client --keepalive 20s --max-retry-interval 30s \
      "https://$FQDN/_gpu" "R:0.0.0.0:11436:${OLLAMA#http://}" >> "$LOG" 2>&1 &
    echo $! > "$PIDFILE"
    sleep 3
    if running; then echo "tunnel started (pid $(cat "$PIDFILE")), log $LOG"; else echo "tunnel exited; see $LOG" >&2; tail -n 5 "$LOG" >&2; exit 1; fi ;;
  down)
    if running; then kill "$(cat "$PIDFILE")" && rm -f "$PIDFILE" && echo "tunnel stopped"; else echo "not running"; fi ;;
  status)
    if running; then echo "client running (pid $(cat "$PIDFILE"))"; else echo "client not running"; exit 1; fi
    tail -n 3 "$LOG" ;;
  *) echo "usage: $0 install|up|down|status" >&2; exit 2 ;;
esac
