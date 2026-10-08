#!/usr/bin/env bash
# Fetch the llama.cpp converter (HF safetensors to GGUF) at a pinned release into
# slm-distill/.tools/llama.cpp. Only the converter and its gguf package are unpacked.
set -euo pipefail
cd "$(dirname "$0")/.."
TAG=v0.6.0
SHA256=09b36dba235fcac180efff18514da7038e65d8e5b9e4aefa59238468abfdec12
DEST=.tools/llama.cpp
if [ -f "$DEST/convert_hf_to_gguf.py" ] && [ "$(cat "$DEST/.tag" 2>/dev/null)" = "$TAG" ]; then
  echo "llama.cpp $TAG already in $DEST"; exit 0
fi
mkdir -p .tools
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT
curl -fsSL -o "$tmp" "https://github.com/ggml-org/llama.cpp/archive/refs/tags/$TAG.tar.gz"
echo "$SHA256  $tmp" | sha256sum -c - >/dev/null || { echo "checksum mismatch for llama.cpp $TAG" >&2; exit 1; }
rm -rf "$DEST"; mkdir -p "$DEST"
tar xzf "$tmp" -C "$DEST" --strip-components=1 \
  "llama.cpp-${TAG#v}/convert_hf_to_gguf.py" "llama.cpp-${TAG#v}/conversion" "llama.cpp-${TAG#v}/gguf-py"
echo "$TAG" > "$DEST/.tag"
echo "llama.cpp $TAG unpacked in $DEST"
