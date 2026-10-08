"""Turn the LoRA adapter into a model Ollama serves.

    python export.py --base Qwen/Qwen3-4B-Instruct-2507 --adapter out/local-4b-general/final --name mem-extractor:general-v1 --quant q8_0

Steps: merge the adapter into the bf16 base, convert to GGUF with the pinned llama.cpp converter
(tools/fetch-llamacpp.sh), then create through Ollama's API with the chat
format used in training (system and user messages; original Qwen3 adds an empty think block, while Instruct-2507 does not). The model
file also carries the SYSTEM text, but the engine sends it with every call anyway.
With --base-only the base model is exported unchanged, as the zero-shot reference for evaluation.
Set OLLAMA_HOST to the server that should hold the model.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import urllib.request
import urllib.error
from pathlib import Path

import prompt

HERE = Path(__file__).resolve().parent
BASE = "Qwen/Qwen3-1.7B"

# The chat format used in training: system message, user message, and Qwen3's empty think block.
TEMPLATE = """{{- if .System }}<|im_start|>system
{{ .System }}<|im_end|>
{{ end }}<|im_start|>user
{{ .Prompt }}<|im_end|>
<|im_start|>assistant
<think>

</think>

"""
STOP = ["<|im_end|>", "<|endoftext|>"]
SPEC_FILE = HERE / "ollama-model.json"


def spec() -> dict:
    """What turns a bare GGUF into the served model. scripts/fetch-slm.sh sends this to Ollama's
    /api/create, so a machine without this repository's Python setup can build the same model."""
    return {"template": TEMPLATE, "system": prompt.SYSTEM, "parameters": {"stop": STOP, "temperature": 0}}


def modelfile(gguf_name: str = "model-f16.gguf") -> str:
    s = spec()
    return (f'FROM ./{gguf_name}\nTEMPLATE """{s["template"]}"""\nSYSTEM """{s["system"].replace(chr(34) * 3, chr(39) * 3)}"""\n'
            + "".join(f'PARAMETER stop "{x}"\n' for x in STOP) + "PARAMETER temperature 0\n")


def run(cmd: list[str], **kwargs) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, **kwargs)


def served_gguf(base: str, model: str) -> Path:
    request = urllib.request.Request(base + "/api/show", data=json.dumps({"model": model}).encode(),
                                     headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        info = json.load(response)
    source = next(line[5:].strip().strip('"') for line in info["modelfile"].splitlines()
                  if line.startswith("FROM "))
    path = Path(source)
    if not path.is_file():
        raise RuntimeError("--gguf-out requires local access to the Ollama server's served model blob")
    return path


def create_uploaded_model(base: str, body: dict, recovery_file: Path) -> dict:
    """Preserve the exact import request so a server failure never requires remerging."""
    recovery_file.write_text(json.dumps(body, indent=2) + "\n")
    digest = ", ".join(body["files"].values())
    recovery = (f"Uploaded blob {digest} remains on this Ollama server. "
                f"After resolving the server error, POST {recovery_file} as JSON to /api/create "
                "on the same server. Reuse the uploaded blob; do not remerge or retrain.")
    request = urllib.request.Request(base + "/api/create", data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=600) as response:
            result = json.load(response)
    except urllib.error.HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")[:8192]
        raise RuntimeError(f"Ollama import HTTP {exc.code}: {detail}\n{recovery}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(f"Ollama import connection failed: {exc.reason}\n{recovery}") from exc
    if result.get("status") != "success":
        raise RuntimeError(f"Ollama import failed: {result}\n{recovery}")
    return result


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=BASE)
    ap.add_argument("--revision")
    ap.add_argument("--write-spec", action="store_true", help="write ollama-model.json and stop")
    ap.add_argument("--gguf-out", type=Path, help="also copy the created model's GGUF here (for publishing)")
    ap.add_argument("--adapter", type=Path)
    ap.add_argument("--base-only", action="store_true")
    ap.add_argument("--name", help="Ollama model name to create")
    ap.add_argument("--quant", default="q8_0", help="q8_0, q4_K_M, f16 ...")
    ap.add_argument("--work", type=Path, default=HERE / "out" / "export")
    ap.add_argument("--keep-work", action="store_true")
    args = ap.parse_args()
    global TEMPLATE
    if "Instruct-2507" in args.base:
        TEMPLATE = TEMPLATE.split("<think>")[0]
    if args.write_spec:
        SPEC_FILE.write_text(json.dumps(spec(), indent=1) + "\n")
        print("wrote", SPEC_FILE)
        return
    if not args.name:
        sys.exit("--name is required")
    if bool(args.adapter) == args.base_only:
        sys.exit("give exactly one of --adapter or --base-only")

    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer

    converter = HERE / ".tools" / "llama.cpp" / "convert_hf_to_gguf.py"
    if not converter.exists():
        sys.exit("converter missing: run slm-distill/tools/fetch-llamacpp.sh first")
    merged = args.work / args.name.replace(":", "_")
    shutil.rmtree(merged, ignore_errors=True)
    merged.mkdir(parents=True)

    model = AutoModelForCausalLM.from_pretrained(args.base, revision=args.revision, dtype=torch.bfloat16)
    if args.adapter:
        from peft import PeftModel
        model = PeftModel.from_pretrained(model, str(args.adapter)).merge_and_unload()
    model.save_pretrained(merged, safe_serialization=True)
    AutoTokenizer.from_pretrained(args.base, revision=args.revision).save_pretrained(merged)
    del model

    env = {**os.environ, "PYTHONPATH": str(HERE / ".tools" / "llama.cpp" / "gguf-py")}
    # Q8_0 is supported by the pinned converter. Quantize there to avoid storing
    # two additional full-precision model copies during Ollama import.
    outtype = "q8_0" if args.quant == "q8_0" else "f16"
    gguf = merged / f"model-{outtype}.gguf"
    run([sys.executable, str(converter), str(merged), "--outfile", str(gguf), "--outtype", outtype], env=env)
    if not args.keep_work:
        for weights in merged.glob("*.safetensors"):
            weights.unlink()
    (merged / "Modelfile").write_text(modelfile(gguf.name))
    # Upload before creating so the local duplicate can be released before
    # Ollama's compatibility check allocates another temporary model copy.
    base = os.environ.get("OLLAMA_HOST", "http://127.0.0.1:11434").rstrip("/")
    if "://" not in base:
        base = "http://" + base
    digest = "sha256:" + hashlib.file_digest(gguf.open("rb"), "sha256").hexdigest()
    with gguf.open("rb") as weights:
        request = urllib.request.Request(base + "/api/blobs/" + digest, data=weights, method="POST",
                                         headers={"Content-Length": str(gguf.stat().st_size)})
        with urllib.request.urlopen(request, timeout=600):
            pass
    print(f"uploaded model blob {digest}", flush=True)
    if not args.keep_work:
        gguf.unlink()
    body = {"model": args.name, "files": {gguf.name: digest}, "stream": False, **spec()}
    if args.quant not in ("f16", "q8_0"):
        body["quantize"] = args.quant
    create_uploaded_model(base, body, merged / "ollama-create-recovery.json")
    if args.gguf_out:
        args.gguf_out.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(served_gguf(base, args.name), args.gguf_out)
        print(f"copied the served GGUF to {args.gguf_out}")
    if not args.keep_work:
        shutil.rmtree(merged)
    print(f"created {args.name} ({args.quant}) on {os.environ.get('OLLAMA_HOST', 'the default Ollama host')}")


if __name__ == "__main__":
    main()
