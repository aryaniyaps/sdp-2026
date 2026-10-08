#!/usr/bin/env bash
# Installs the memory worker model into an Ollama server, over Ollama's HTTP API (no Ollama CLI
# and no Python training setup needed). The model is the fine-tuned Qwen3-4B-Instruct-2507 student; its weights
# and checksums are published on Hugging Face.
#
#   scripts/fetch-slm.sh                          download, check, and create `mem-extractor`
#   SLM_FILE=/path/to/model.gguf scripts/fetch-slm.sh    use a GGUF you already have
#
#   OLLAMA_URL       the Ollama server to install into (default http://127.0.0.1:11434)
#   EXTRACTION_MODEL name to create (default mem-extractor)
#   SLM_REVISION     Hugging Face revision to download (default pinned release commit)
#   SLM_REPO         Hugging Face model repository (default aryaniyaps/mem-extractor)
#   SLM_BASE_URL     optional download mirror (must serve weights and SHA256SUMS)
#   SLM_CACHE        where the download is kept (default ~/.cache/mem-extractor)
#   FORCE=1          create again even when the model already exists
set -euo pipefail
cd "$(dirname "$0")/.."
OLLAMA_URL=${OLLAMA_URL:-http://127.0.0.1:11434}
MODEL=${EXTRACTION_MODEL:-mem-extractor}
REVISION=${SLM_REVISION:-77851da91c15fd4e20018a1b0ace13d29e7eb635}
REPO=${SLM_REPO:-aryaniyaps/mem-extractor}
ASSET=mem-extractor-q8_0.gguf
CACHE=${SLM_CACHE:-$HOME/.cache/mem-extractor}
BASE_URL=${SLM_BASE_URL:-https://huggingface.co/$REPO/resolve/$REVISION}

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
    || { echo "cannot download $BASE_URL/SHA256SUMS; is $REPO at $REVISION published?" >&2; exit 1; }
  want=$(awk -v f="$ASSET" '$2==f {print $1}' "$CACHE/SHA256SUMS")
  [[ "$want" =~ ^[0-9a-fA-F]{64}$ ]] || { echo "$ASSET is not listed in the release checksums (one SHA-256 entry required)" >&2; exit 1; }
  if [ ! -f "$file" ] || [ "$(sha256sum "$file" | cut -d' ' -f1)" != "$want" ]; then
    echo "Downloading $ASSET (about 4.3 GB) from $REPO at $REVISION"
    curl -fL --retry 3 -o "$file.part" "$BASE_URL/$ASSET"
    got=$(sha256sum "$file.part" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { echo "checksum mismatch for downloaded $ASSET" >&2; rm -f "$file.part"; exit 1; }
    mv "$file.part" "$file"
  fi
fi
digest=$(sha256sum "$file" | cut -d' ' -f1)
if [ -z "${SLM_FILE:-}" ] && [ "$digest" != "$want" ]; then
  echo "checksum mismatch for $file: expected $want, got $digest" >&2
  exit 1
fi

echo "Sending the weights to $OLLAMA_URL"
curl -fsS -X POST --data-binary @"$file" "$OLLAMA_URL/api/blobs/sha256:$digest" > /dev/null
create_json=$(mktemp "${TMPDIR:-/tmp}/mem-extractor-create.XXXXXXXX.json")
trap 'rm -f "$create_json"' EXIT
python3 - "$digest" "$MODEL" > "$create_json" <<'PY'
import json, sys
digest, model = sys.argv[1:3]
spec = json.load(open("slm-distill/ollama-model.json"))
print(json.dumps({"model": model, "files": {"model.gguf": f"sha256:{digest}"}, "stream": False, **spec}))
PY
curl -fsS --max-time 600 "$OLLAMA_URL/api/create" -d @"$create_json"
echo
curl -fsS --max-time 10 "$OLLAMA_URL/api/show" -d "{\"model\":\"$MODEL\"}" > /dev/null
echo "$MODEL is ready at $OLLAMA_URL"
