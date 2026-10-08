#!/usr/bin/env bash
# Creates the hosted setup from nothing: Pi in a browser terminal next to the live memory graph, on
# one Azure VM, behind HTTPS and one access code. Run it once; afterwards deploy.sh updates the app.
# It refuses to run when the VM already exists.
#
# Needs: az (logged in), docker (only to hash the access code), openssl, ssh-keygen, tar, base64,
# and a Pi that is logged in on this machine (its auth.json is copied to the VM, for the terminal only:
# the memory worker is the fine-tuned model and never uses it).
#   DNS_LABEL     required, a name unique in the region: the page is <label>.<location>.cloudapp.azure.com
#   AZURE_RG      resource group            (default sdp-memory-rg)
#   AZURE_LOC     location                  (default eastus)
#   AZURE_VM      virtual machine           (default sdp-memory-vm)
#   VM_SIZE       size                      (default Standard_D2s_v4, 2 vCPU 8 GB; a size can be unavailable in a subscription)
#   PI_AGENT_DIR  Pi's agent directory      (default ~/.pi/agent)
#   PI_PROVIDER, PI_MODEL                   (default: Pi's settings.json, used by the terminal only)
#   GPU_TUNNEL    1 (default) lets a workstation's GPU serve the worker model through gpu-tunnel.sh;
#                 0 uses the VM's own Ollama (CPU, slow) and you install the model there yourself
#   SEED_DUMP     a dump made by make-seed.sh; the memory starts from it (default: start empty)
#   SSH_FROM      a CIDR to open port 22 for (default: SSH stays closed, nothing here needs it)
#   SDP_CLOUD_STATE  where the access code, passwords and the admin key are kept (default ~/.sdp-cloud)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
# shellcheck disable=SC2034
REPO=$(git -C "$HERE" rev-parse --show-toplevel)
RG=${AZURE_RG:-sdp-memory-rg}
LOC=${AZURE_LOC:-eastus}
VM=${AZURE_VM:-sdp-memory-vm}
SIZE=${VM_SIZE:-Standard_D2s_v4}
DNS=${DNS_LABEL:?set DNS_LABEL to a name nobody else uses in the region, for example sdp-memory-1234}
STATE=${SDP_CLOUD_STATE:-$HOME/.sdp-cloud}
PI_DIR=${PI_AGENT_DIR:-$HOME/.pi/agent}
SEED=${SEED_DUMP:-}
FQDN=$DNS.$LOC.cloudapp.azure.com
# shellcheck source=lib.sh
. "$HERE/lib.sh"

for tool in az docker openssl ssh-keygen tar base64 python3; do
  command -v "$tool" > /dev/null || { echo "missing tool: $tool" >&2; exit 1; }
done
az account show > /dev/null || { echo "run az login first" >&2; exit 1; }
[ -f "$PI_DIR/auth.json" ] || { echo "no $PI_DIR/auth.json: log in with Pi first, the hosted terminal and the memory worker use that login" >&2; exit 1; }
read -r PI_PROVIDER PI_MODEL < <(python3 "$HERE/pi-settings.py" "$PI_DIR/settings.json")
[ -z "$SEED" ] || [ -f "$SEED" ] || { echo "SEED_DUMP $SEED does not exist" >&2; exit 1; }
if [ -n "$SEED" ] && [ "$(wc -c < "$SEED")" -gt 20000000 ]; then
  echo "SEED_DUMP is $(( $(wc -c < "$SEED") / 1000000 )) MB. It travels through az vm run-command in 150 KB parts (about 12 seconds each), so this would take over an hour. Make a smaller seed, or start empty." >&2
  exit 1
fi
if az vm show -g "$RG" -n "$VM" > /dev/null 2>&1; then
  echo "VM $VM already exists in $RG. Use deploy.sh to update it, or az group delete -n $RG to start over." >&2
  exit 1
fi

umask 077
mkdir -p "$STATE"
if [ ! -f "$STATE/secrets.env" ]; then
  {
    echo "POSTGRES_PASSWORD=$(openssl rand -hex 16)"
    echo "NEO4J_PASSWORD=$(openssl rand -hex 16)"
    echo "ACCESS_CODE=$(openssl rand -hex 12)"
    echo "GPU_TUNNEL_SECRET=$(openssl rand -hex 24)"
  } > "$STATE/secrets.env"
fi
grep -q '^GPU_TUNNEL_SECRET=' "$STATE/secrets.env" || echo "GPU_TUNNEL_SECRET=$(openssl rand -hex 24)" >> "$STATE/secrets.env"
get() { grep -m1 "^$1=" "$STATE/secrets.env" | cut -d= -f2-; }
[ -f "$STATE/vm_key" ] || ssh-keygen -t ed25519 -N '' -C "$VM" -f "$STATE/vm_key" -q
# shellcheck disable=SC2034  # read by render_config in lib.sh
hash=$(docker run --rm caddy:2 caddy hash-password --plaintext "$(get ACCESS_CODE)")

echo "Creating $RG, the network rules and $VM ($SIZE)"
az group create -n "$RG" -l "$LOC" -o none
az network nsg create -g "$RG" -n sdp-memory-nsg -l "$LOC" -o none
az network nsg rule create -g "$RG" --nsg-name sdp-memory-nsg -n web80 --priority 100 --access Allow --protocol Tcp \
  --direction Inbound --destination-port-ranges 80 --source-address-prefixes Internet -o none
az network nsg rule create -g "$RG" --nsg-name sdp-memory-nsg -n web443 --priority 110 --access Allow --protocol Tcp \
  --direction Inbound --destination-port-ranges 443 --source-address-prefixes Internet -o none
if [ -n "${SSH_FROM:-}" ]; then
  az network nsg rule create -g "$RG" --nsg-name sdp-memory-nsg -n ssh-me --priority 120 --access Allow --protocol Tcp \
    --direction Inbound --destination-port-ranges 22 --source-address-prefixes "$SSH_FROM" -o none
fi
az vm create -g "$RG" -n "$VM" --image Canonical:ubuntu-24_04-lts:server:latest --size "$SIZE" \
  --admin-username azureuser --ssh-key-values "$STATE/vm_key.pub" \
  --public-ip-address-dns-name "$DNS" --public-ip-sku Standard --os-disk-size-gb 64 --storage-sku Premium_LRS \
  --custom-data "$HERE/cloud-init.yaml" --nsg sdp-memory-nsg --nsg-rule NONE -o none

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

echo "Waiting for Docker to be installed on the VM"
cat > "$WORK/wait.sh" <<'SH'
cloud-init status --wait > /dev/null 2>&1 || true
docker --version
test -d /opt/sdp
echo STEP_OK
SH
for attempt in 1 2 3 4 5 6; do
  on_vm "$WORK/wait.sh" && break
  [ "$attempt" = 6 ] && { echo "the VM never became ready" >&2; exit 1; }
  sleep 20
done

echo "Preparing the configuration"
cfg=$WORK/cfg
mkdir -p "$cfg/pi-agent"
render_config "$cfg"
cp "$PI_DIR/auth.json" "$cfg/pi-agent/auth.json"
printf '{\n  "defaultProvider": "%s",\n  "defaultModel": "%s",\n  "defaultThinkingLevel": "medium",\n  "quietStartup": true,\n  "tuiMode": "regular"\n}\n' \
  "$PI_PROVIDER" "$PI_MODEL" > "$cfg/pi-agent/settings.json"
[ -z "$SEED" ] || cp "$SEED" "$cfg/seed.dump"
tar -C "$cfg" -czf "$WORK/cfg.tgz" .
echo "Uploading the configuration ($(wc -c < "$WORK/cfg.tgz") bytes)"
put_file "$WORK/cfg.tgz" /opt/sdp/config.tgz

cat > "$WORK/unpack.sh" <<'SH'
set -eu
cd /opt/sdp
tar xzf config.tgz && rm -f config.tgz
# The files were made under umask 077. The terminal runs pi-session.sh as uid 10001, so it must be readable by everyone.
chmod 755 pi-session.sh reset-seed.sh
chmod 644 docker-compose.yml Caddyfile
chmod 755 conf.d && chmod 644 conf.d/* 2>/dev/null || true
chmod 600 .env gpu-tunnel-users.json 2>/dev/null || true
# The terminal runs as uid 10001 inside the image and reads the Pi login from here.
chown -R 10001:10001 pi-agent && chmod 700 pi-agent && chmod 600 pi-agent/auth.json
# Start the stores and pull the embedding model before the first app image exists.
docker compose up -d postgres neo4j ollama
docker compose up ollama-init 2>&1 | tail -n 3
echo STEP_OK
SH
echo "Starting the databases and pulling the embedding model"
on_vm "$WORK/unpack.sh"

echo "Building and starting the app"
"$HERE/deploy.sh"

if [ -n "$SEED" ]; then
  echo "Restoring the seed memory"
  printf 'set -eu\ncd /opt/sdp\n./reset-seed.sh\necho STEP_OK\n' > "$WORK/seed.sh"
  on_vm "$WORK/seed.sh"
fi

cat > "$WORK/finish.sh" <<'SH'
set -eu
cd /opt/sdp
docker compose up -d
docker compose ps --format 'table {{.Service}}\t{{.Status}}'
# The Pi login and the passwords travelled inside run-command scripts. Remove the copies the Azure agent keeps.
rm -rf /var/lib/waagent/run-command/download/*
echo STEP_OK
SH
echo "Starting the proxy"
on_vm "$WORK/finish.sh"

cat <<DONE

Done. Page:   https://$FQDN/   (the certificate can take a minute)
User:         admin
Access code:  $STATE/secrets.env (ACCESS_CODE)
Later:        deploy/azure/deploy.sh updates the app; az vm deallocate -g $RG -n $VM stops the compute cost;
              az group delete -n $RG removes everything.
DONE
