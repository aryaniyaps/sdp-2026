#!/usr/bin/env bash
# Installs the memory worker model into an Ollama server, over Ollama's HTTP API (no Ollama CLI
# and no Python training setup needed). The model is the fine-tuned Qwen3-1.7B student; its weights
# are a GitHub release asset because they are too large for the repository.
#
#   scripts/fetch-slm.sh                          download, check, and create `memex-extractor`
#   SLM_FILE=/path/to/model.gguf scripts/fetch-slm.sh    use a GGUF you already have
#
#   OLLAMA_URL       the Ollama server to install into (default http://127.0.0.1:11434)
#   EXTRACTION_MODEL name to create (default memex-extractor)
#   SLM_RELEASE      release tag to download from (default slm-v1)
#   SLM_CACHE        where the download is kept (default ~/.cache/memex-slm)
#   FORCE=1          create again even when the model already exists
set -euo pipefail
cd "$(dirname "$0")/.."
OLLAMA_URL=${OLLAMA_URL:-http://127.0.0.1:11434}
MODEL=${EXTRACTION_MODEL:-memex-extractor}
RELEASE=${SLM_RELEASE:-slm-v1}
ASSET=memex-extractor-q8_0.gguf
CACHE=${SLM_CACHE:-$HOME/.cache/memex-slm}
BASE_URL=https://github.com/aryaniyaps/sdp-2026/releases/download/$RELEASE

curl -fsS --max-time 5 "$OLLAMA_URL/api/version" > /dev/null \
  || { echo "No Ollama server answers at $OLLAMA_URL. Start one, or set OLLAMA_URL." >&2; exit 1; }
if [ "${FORCE:-0}" != 1 ] && curl -fsS --max-time 10 "$OLLAMA_URL/api/show" -d "{\"model\":\"$MODEL\"}" > /dev/null 2>&1; then
  echo "$MODEL is already installed at $OLLAMA_URL (FORCE=1 to create it again)"
  exit 0
fi

file=${SLM_FILE:-}
if [ -z "$file" ]; then
  mkdir -p "$CACHE"
  file=$CACHE/$ASSET
  curl -fsSL -o "$CACHE/SHA256SUMS" "$BASE_URL/SHA256SUMS" \
    || { echo "cannot download $BASE_URL/SHA256SUMS; is release $RELEASE published?" >&2; exit 1; }
  want=$(awk -v f="$ASSET" '$2==f {print $1}' "$CACHE/SHA256SUMS")
  [ -n "$want" ] || { echo "$ASSET is not listed in the release checksums" >&2; exit 1; }
  if [ ! -f "$file" ] || [ "$(sha256sum "$file" | cut -d' ' -f1)" != "$want" ]; then
    echo "Downloading $ASSET (about 1.8 GB) from release $RELEASE"
    curl -fL --retry 3 -C - -o "$file" "$BASE_URL/$ASSET"
  fi
fi
digest=$(sha256sum "$file" | cut -d' ' -f1)
if [ -z "${SLM_FILE:-}" ] && [ "$digest" != "$want" ]; then
  echo "checksum mismatch for $file: expected $want, got $digest" >&2
  exit 1
fi

echo "Sending the weights to $OLLAMA_URL"
curl -fsS -X POST --data-binary @"$file" "$OLLAMA_URL/api/blobs/sha256:$digest" > /dev/null
python3 - "$digest" "$MODEL" > /tmp/memex-create.$$.json <<'PY'
import json, sys
digest, model = sys.argv[1:3]
spec = json.load(open("slm-distill/ollama-model.json"))
print(json.dumps({"model": model, "files": {"model.gguf": f"sha256:{digest}"}, "stream": False, **spec}))
PY
trap 'rm -f /tmp/memex-create.$$.json' EXIT
curl -fsS --max-time 600 "$OLLAMA_URL/api/create" -d @/tmp/memex-create.$$.json
echo
curl -fsS --max-time 10 "$OLLAMA_URL/api/show" -d "{\"model\":\"$MODEL\"}" > /dev/null
echo "$MODEL is ready at $OLLAMA_URL"
