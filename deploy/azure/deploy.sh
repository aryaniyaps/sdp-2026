#!/usr/bin/env bash
# Deploys the checked-out source to the Azure VM without SSH: the source travels through
# `az vm run-command`, the image is built on the VM, then the engine and term containers are
# swapped to it. If the new engine is not healthy the previous image is put back and this fails.
# Needs: az (logged in), git, tar, xz, base64. Run from anywhere inside the repository.
#   AZURE_RG  resource group   (default sdp-memory-rg)
#   AZURE_VM  virtual machine  (default sdp-memory-vm)
set -euo pipefail

RG=${AZURE_RG:-sdp-memory-rg}
VM=${AZURE_VM:-sdp-memory-vm}
REPO=$(git rev-parse --show-toplevel)
TAG=$(git -C "$REPO" rev-parse --short=12 HEAD)
# A tag that names a commit must mean that commit: mark a build of uncommitted changes.
[ -z "$(git -C "$REPO" status --porcelain -- src migrations integrations/pi Cargo.toml Cargo.lock deploy/azure)" ] || TAG="$TAG-dirty"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# Runs a script on the VM as root and fails unless it reached its last line. run-command reports
# success even when the script failed, so the script must end by printing STEP_OK.
on_vm() {
  local out
  out=$(az vm run-command invoke -g "$RG" -n "$VM" --command-id RunShellScript --scripts @"$1" \
    --query 'value[0].message' -o tsv)
  printf '%s\n' "$out" | cut -c1-300
  printf '%s' "$out" | grep -q '^STEP_OK$' || { echo "deploy failed on the VM (step $1)" >&2; exit 1; }
}

echo "Packing $TAG"
ctx=$WORK/ctx
mkdir -p "$ctx/deploy/azure" "$ctx/integrations"
cp "$REPO/Cargo.toml" "$REPO/Cargo.lock" "$REPO/rust-toolchain.toml" "$ctx/"
cp -r "$REPO/migrations" "$REPO/src" "$ctx/"
cp -r "$REPO/integrations/pi" "$ctx/integrations/pi"
rm -rf "$ctx/integrations/pi/node_modules" "$ctx/integrations/pi/test"
cp "$REPO/deploy/azure/Dockerfile" "$ctx/Dockerfile"
cp "$REPO/deploy/azure/pi-session.sh" "$ctx/deploy/azure/pi-session.sh"
tar -C "$ctx" -cJf "$WORK/ctx.txz" .
base64 -w0 "$WORK/ctx.txz" > "$WORK/ctx.b64"
echo "Source archive: $(wc -c < "$WORK/ctx.txz") bytes"

# run-command scripts are limited to 256 KB, so the archive goes over in chunks.
split -b 150000 -d "$WORK/ctx.b64" "$WORK/chunk."
first=1
for c in "$WORK"/chunk.*; do
  {
    echo 'set -eu'
    if [ $first = 1 ]; then echo ': > /opt/sdp/deploy-ctx.b64'; fi
    echo "cat >> /opt/sdp/deploy-ctx.b64 <<'CHUNK'"
    cat "$c"; echo
    echo 'CHUNK'
    echo 'echo STEP_OK'
  } > "$WORK/send.sh"
  first=0
  echo "Uploading $(basename "$c")"
  on_vm "$WORK/send.sh" > /dev/null
done

cat > "$WORK/build.sh" <<SH
set -eu
D=/opt/sdp/build-$TAG
rm -rf "\$D" && mkdir -p "\$D"
base64 -d /opt/sdp/deploy-ctx.b64 | tar xJ -C "\$D"
rm -f /opt/sdp/deploy-ctx.b64 /opt/sdp/deploy-build.log
cd "\$D"
nohup sh -c 'docker build -t sdp-memory-app:$TAG . > /opt/sdp/deploy-build.log 2>&1; echo "exit=\$?" >> /opt/sdp/deploy-build.log' >/dev/null 2>&1 &
echo STEP_OK
SH
echo "Building sdp-memory-app:$TAG on $VM"
on_vm "$WORK/build.sh" > /dev/null

cat > "$WORK/wait.sh" <<'SH'
timeout 270 sh -c 'until grep -q "^exit=" /opt/sdp/deploy-build.log; do sleep 10; done' || true
if grep -q '^exit=0$' /opt/sdp/deploy-build.log; then echo BUILD_DONE; fi
if grep -q '^exit=[1-9]' /opt/sdp/deploy-build.log; then echo BUILD_FAILED; tail -n 25 /opt/sdp/deploy-build.log | cut -c1-200; fi
echo STEP_OK
SH
for _ in $(seq 1 12); do
  out=$(az vm run-command invoke -g "$RG" -n "$VM" --command-id RunShellScript --scripts @"$WORK/wait.sh" \
    --query 'value[0].message' -o tsv)
  if printf '%s' "$out" | grep -q BUILD_FAILED; then printf '%s\n' "$out" >&2; echo "image build failed, nothing was changed" >&2; exit 1; fi
  if printf '%s' "$out" | grep -q BUILD_DONE; then built=1; break; fi
done
[ "${built:-0}" = 1 ] || { echo "image build did not finish within an hour, nothing was changed" >&2; exit 1; }

cat > "$WORK/swap.sh" <<SH
set -u
cd /opt/sdp
prev=\$(grep -m1 -o 'sdp-memory-app:[A-Za-z0-9._-]*' docker-compose.yml)
cp docker-compose.yml docker-compose.yml.previous
sed -i 's#image: sdp-memory-app:[A-Za-z0-9._-]*#image: sdp-memory-app:$TAG#' docker-compose.yml
docker compose up -d engine term 2>&1 | tail -n 4
ok=0
for _ in \$(seq 1 40); do
  if docker compose exec -T engine curl -fsS -o /dev/null 'http://127.0.0.1:8080/api/v2/status?namespace=deploy-check'; then ok=1; break; fi
  sleep 3
done
if [ \$ok = 1 ]; then
  echo "healthy on sdp-memory-app:$TAG (was \$prev)"
  rm -rf /opt/sdp/build-$TAG
  docker images sdp-memory-app --format '{{.Tag}}' | tail -n +4 | while read -r t; do docker rmi "sdp-memory-app:\$t" >/dev/null 2>&1 && echo "removed old image \$t"; done
  echo STEP_OK
else
  echo "engine not healthy, putting \$prev back"
  docker compose logs engine --tail 20 2>&1 | cut -c1-200
  cp docker-compose.yml.previous docker-compose.yml
  docker compose up -d engine term 2>&1 | tail -n 2
fi
SH
echo "Switching engine and term to $TAG"
on_vm "$WORK/swap.sh"
echo "Deployed $TAG"
