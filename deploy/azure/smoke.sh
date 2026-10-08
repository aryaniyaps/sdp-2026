#!/usr/bin/env bash
# Runs scripts/worker-smoke.py on the VM against the hosted engine, which is not reachable from
# outside. It retains two small coding-session episodes in a throwaway namespace, waits for the
# worker jobs (so the model server, through the GPU tunnel or the VM's own Ollama, must be up) and
# checks recall, the correction and that a planted credential is not stored. Calls the real model.
# Needs: az (logged in) and the state that provision.sh left in ~/.sdp-cloud.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(git -C "$HERE" rev-parse --show-toplevel)
STATE=${SDP_CLOUD_STATE:-$HOME/.sdp-cloud}
# shellcheck disable=SC1091
. "$STATE/azure.env"
# shellcheck source=lib.sh
. "$HERE/lib.sh"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

put_file "$REPO/scripts/worker-smoke.py" /opt/sdp/worker-smoke.py
cat > "$WORK/run.sh" <<'SH'
set -eu
cd /opt/sdp
ip=$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$(docker compose ps -q engine)")
python3 /opt/sdp/worker-smoke.py --url "http://$ip:8080" --timeout 900
echo STEP_OK
SH
on_vm "$WORK/run.sh"
