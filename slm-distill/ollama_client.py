"""Call a model the way the engine does (src/model.rs OllamaJsonModel), for evaluation."""

from __future__ import annotations

import json
import time
import urllib.request

import prompt
import structured


def generate(base: str, model: str, user: str, *, num_ctx: int = 12288, num_predict: int = 3072,
             timeout: int = 600) -> dict:
    """POST /api/generate with the engine's request body. Returns the parsed response plus wall seconds."""
    user, schema, paired = structured.prepare(user)
    if prompt.estimate_tokens(user) + num_predict > num_ctx:
        raise ValueError("structured prompt exceeds context budget")
    body = {
        "model": model, "system": prompt.SYSTEM, "prompt": user, "format": schema, "think": False, "stream": False,
        "keep_alive": "1h", "options": {"num_ctx": num_ctx, "num_predict": num_predict, "temperature": 0},
    }
    request = urllib.request.Request(f"{base}/api/generate", data=json.dumps(body).encode(),
                                     headers={"content-type": "application/json"})
    started = time.time()
    with urllib.request.urlopen(request, timeout=timeout) as response:
        out = json.load(response)
    if paired:
        out["wire_response"] = out["response"]
        out["response"] = json.dumps(structured.canonicalize(json.loads(out["response"]), paired))
    out["wall_seconds"] = time.time() - started
    return out


def embed(base: str, model: str, texts: list[str]) -> list[list[float]]:
    request = urllib.request.Request(
        f"{base}/api/embed", data=json.dumps({"model": model, "input": texts}).encode(),
        headers={"content-type": "application/json"})
    with urllib.request.urlopen(request, timeout=300) as response:
        return json.load(response)["embeddings"]
