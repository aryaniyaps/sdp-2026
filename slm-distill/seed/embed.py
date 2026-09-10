"""Embed seed statements with Azure text-embedding-3-small at 1024 dimensions.

1024 is not a truncation hack: text-embedding-3 models are trained with Matryoshka
representation learning, so a shortened vector is still a valid embedding. It is
chosen here because migrations/0001 declares vector(1024).

Results are cached to seed/embeddings.jsonl so this only ever runs once.

  python seed/embed.py
"""

import json
import time
from pathlib import Path

from openai import AzureOpenAI

ROOT = Path(__file__).resolve().parent.parent
DEPLOYMENT = "text-embedding-3-small"
DIMS = 1024
BATCH = 64


def load_env():
    env = {}
    for line in (ROOT / ".env").read_text().splitlines():
        if "=" in line and not line.strip().startswith("#"):
            k, _, v = line.partition("=")
            env[k.strip()] = v.strip()
    return env


def main():
    todo = json.loads((ROOT / "seed" / "to_embed.json").read_text())
    out = ROOT / "seed" / "embeddings.jsonl"
    done = set()
    if out.exists():
        for line in out.read_text().splitlines():
            try:
                done.add(json.loads(line)["t"])
            except Exception:
                pass
    todo = [t for t in todo if t and t not in done]
    print(f"to embed: {len(todo)}  (cached {len(done)})")
    if not todo:
        return

    env = load_env()
    client = AzureOpenAI(api_key=env["AZURE_OPENAI_KEY"],
                         azure_endpoint=env["AZURE_OPENAI_ENDPOINT"],
                         api_version="2024-10-21", timeout=120.0)

    t0 = time.time()
    with out.open("a") as f:
        for i in range(0, len(todo), BATCH):
            batch = todo[i:i + BATCH]
            for attempt in range(6):
                try:
                    r = client.embeddings.create(model=DEPLOYMENT, input=batch,
                                                 dimensions=DIMS)
                    for text, d in zip(batch, r.data):
                        assert len(d.embedding) == DIMS, len(d.embedding)
                        f.write(json.dumps({"t": text, "v": [round(x, 6)
                                                             for x in d.embedding]}) + "\n")
                    f.flush()
                    break
                except Exception as e:
                    if attempt == 5:
                        raise
                    wait = 15 if "429" in str(e) else 2 ** attempt
                    print(f"  retry ({str(e)[:70]}) in {wait}s", flush=True)
                    time.sleep(wait)
            n = min(i + BATCH, len(todo))
            el = time.time() - t0
            print(f"  {n}/{len(todo)}  {n/el:.0f}/s  eta {(len(todo)-n)/max(n/el,1):.0f}s",
                  flush=True)
    print(f"done in {time.time()-t0:.0f}s")


if __name__ == "__main__":
    main()
