"""Turn the LoRA adapter into a model Ollama serves.

    python export.py --adapter out/worker-lora/final --name memex-extractor --quant q8_0

Steps: merge the adapter into the bf16 base, convert to GGUF with the pinned llama.cpp converter
(tools/fetch-llamacpp.sh), then `ollama create` with a Modelfile whose template is the chat
format used in training (system message, user message, and Qwen3's empty think block). The model
file also carries the SYSTEM text, but the engine sends it with every call anyway.
With --base-only the base model is exported unchanged, as the zero-shot reference for evaluation.
Set OLLAMA_HOST to the server that should hold the model and OLLAMA_BIN to its client.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

import prompt

HERE = Path(__file__).resolve().parent
BASE = "Qwen/Qwen3-1.7B"

MODELFILE = '''FROM ./model-f16.gguf
TEMPLATE """{{- if .System }}<|im_start|>system
{{ .System }}<|im_end|>
{{ end }}<|im_start|>user
{{ .Prompt }}<|im_end|>
<|im_start|>assistant
<think>

</think>

"""
SYSTEM """%s"""
PARAMETER stop "<|im_end|>"
PARAMETER stop "<|endoftext|>"
PARAMETER temperature 0
'''


def run(cmd: list[str], **kwargs) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, **kwargs)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--adapter", type=Path)
    ap.add_argument("--base-only", action="store_true")
    ap.add_argument("--name", required=True, help="Ollama model name to create")
    ap.add_argument("--quant", default="q8_0", help="q8_0, q4_K_M, f16 ...")
    ap.add_argument("--work", type=Path, default=HERE / "out" / "export")
    ap.add_argument("--keep-work", action="store_true")
    args = ap.parse_args()
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

    model = AutoModelForCausalLM.from_pretrained(BASE, dtype=torch.bfloat16)
    if args.adapter:
        from peft import PeftModel
        model = PeftModel.from_pretrained(model, str(args.adapter)).merge_and_unload()
    model.save_pretrained(merged, safe_serialization=True)
    AutoTokenizer.from_pretrained(BASE).save_pretrained(merged)
    del model

    env = {**os.environ, "PYTHONPATH": str(HERE / ".tools" / "llama.cpp" / "gguf-py")}
    gguf = merged / "model-f16.gguf"
    run([sys.executable, str(converter), str(merged), "--outfile", str(gguf), "--outtype", "f16"], env=env)
    (merged / "Modelfile").write_text(MODELFILE % prompt.SYSTEM.replace('"""', "'''"))
    ollama = os.environ.get("OLLAMA_BIN", "ollama")
    create = [ollama, "create", args.name, "-f", "Modelfile"]
    if args.quant != "f16":
        create += ["--quantize", args.quant]
    run(create, cwd=merged)
    if not args.keep_work:
        shutil.rmtree(merged)
    print(f"created {args.name} ({args.quant}) on {os.environ.get('OLLAMA_HOST', 'the default Ollama host')}")


if __name__ == "__main__":
    main()
