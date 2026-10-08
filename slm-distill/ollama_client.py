"""Call a model the way the engine does (src/model.rs OllamaJsonModel), for evaluation."""

from __future__ import annotations

import json
import time
import urllib.request

import prompt


def generate(base: str, model: str, user: str, *, num_ctx: int = 16384, num_predict: int = 4096,
             timeout: int = 600) -> dict:
    """POST /api/generate with the engine's request body. Returns the parsed response plus wall seconds."""
    body = {
        "model": model, "system": prompt.SYSTEM, "prompt": user, "format": "json", "stream": False,
        "keep_alive": "1h", "options": {"num_ctx": num_ctx, "num_predict": num_predict, "temperature": 0},
    }
    request = urllib.request.Request(f"{base}/api/generate", data=json.dumps(body).encode(),
                                     headers={"content-type": "application/json"})
    started = time.time()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        out = json.load(response)
    out["wall_seconds"] = time.time() - started
    return out


def embed(base: str, model: str, texts: list[str]) -> list[list[float]]:
    request = urllib.request.Request(
        f"{base}/api/embed", data=json.dumps({"model": model, "input": texts}).encode(),
        headers={"content-type": "application/json"})
    with urllib.request.urlopen(request, timeout=300) as response:
        return json.load(response)["embeddings"]
