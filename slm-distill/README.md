# mem-extractor: general-purpose memory distillation

This directory trains and evaluates the worker behind the project's **general-purpose conversational memory**: evidence-backed fact extraction, supported observation consolidation, and cited reflection. Personal preferences, relationships, plans, routines, learning and work are in scope; coding is one of sixteen sampled domains.

The released model uses **Qwen3-4B-Instruct-2507**, NF4 QLoRA and a 4,096-token training cap on an RTX 4070 Laptop GPU. It is public at [aryaniyaps/mem-extractor](https://huggingface.co/aryaniyaps/mem-extractor/tree/77851da91c15fd4e20018a1b0ace13d29e7eb635). Final training, matched evaluation and live acceptance are complete, with the evidence and limitations documented below. Release commit `77851da91c15fd4e20018a1b0ace13d29e7eb635` was verified against all 22 staged file hashes and tested for anonymous access; `data/research-v3/huggingface-publication.json` records the checks. Training loss alone is not evidence of better memory quality.

**Measured limitation:** on the internal matched test, consolidation and reflection improve, while extraction coverage declines from 50/55 to 41/55 reference facts. Relative dates and unsupported timestamp precision remain known failure modes. See the protocol for denominators and judging limitations; this is a multitask tradeoff, not a universally better extractor.

## Current experiment and evidence

The general-purpose corpus is frozen at **2,961 task rows**: 2,095 train, 424 validation and 442 test. Every partition covers sixteen domains. Task rows from the same episode are correlated; this is 1,181 episode identifiers including historical replay, not 2,961 independent conversations. Software/technical rows account for 11.46% of training.

The historical 4B coding adapter is followed by general stage A (585 audited rows, completed) and general stage B (1,510 new rows plus 128 replay exposures, completed in 205 optimizer steps). Together A/B cover 2,095 unique training rows. The same teacher family generates and audits targets, so label acceptance is not independent human accuracy.

| Start here | What it contains |
|---|---|
| [Executed iteration notebook](Finetuning_Iteration_Report.ipynb) | Actual failures, changes, training diagnostics and receipt-backed results |
| [HTML notebook](reports/Finetuning_Iteration_Report.html) | Browser-readable version with saved outputs |
| [Research protocol](reports/RESEARCH_PROTOCOL.md) | General-purpose scope, exact corpus counts, lineage, controls and limitations |
| [Prompt design](reports/PROMPT_DESIGN.md) | Hindsight-informed rules, source mapping and measured prompt revisions |
| [External diagnostic](reports/EXTERNAL_LOCOMO.md) | Sixteen held-out LoCoMo windows; not an official LoCoMo QA score |
| [Local project setup](../LOCAL_SETUP.md) | App, databases, Ollama, browser demo and operational verification |

Final evidence lives under `data/research-v3/`: `manifest.json`, `eval/`, `final-acceptance.json`, `release-receipt.json` and publication receipts. The final adapter directory is `out/local-4b-general/`. Large datasets/checkpoints and teacher caches are intentionally excluded from Git. Missing receipts mean the corresponding result remains unverified.

## Setup and reporting

From the repository root, with `uv`, Python 3.12 and a compatible NVIDIA driver:

```bash
bash slm-distill/setup.sh
uv pip install --python slm-distill/.venv/bin/python -r slm-distill/requirements-notebook.txt
env -u PYTHONPATH slm-distill/.venv/bin/python -m ipykernel install --user --name sdp-distill --display-name 'SDP distillation (.venv)'
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/build_notebook.py --execute
env -u PYTHONPATH slm-distill/.venv/bin/jupyter lab slm-distill/Finetuning_Iteration_Report.ipynb
```

Clear externally injected `PYTHONPATH` when running the virtual environment. Notebook Run All refreshes local evidence and figures; it does not start training, call a paid teacher, mutate databases or publish a model.

## Reproduce the current curriculum

The shared runtime prompt files in `../src/worker/`, `../src/v2/` and `../src/model/` are the source of truth. `prompt.py` renders tasks, and `structured.py` applies the paired-source representation used in training and serving. Copying an old prompt into a new dataset breaks the controlled comparison.

The corpus pipeline is:

1. `expand_general_corpus.py`: generate domain/behavior scenarios with partitions assigned by family.
2. `audit_general_corpus.py` and `repair_general_corpus.py`: review labels; repair against unchanged evidence and review again.
3. `assemble_general_corpus.py`: freeze validated partitions, hashes, continuation curriculum and held-out selection.
4. `general_corpus_token_stats.py`: measure actual token lengths and exclusions.
5. `train.py`: train the adapter with answer-only loss, recording exact configuration and completion metrics.

Teacher calls use authenticated Pi and incur provider usage. Credentials are never part of the dataset or model package. Inspect each script's `--help` and preserve frozen inputs before rerunning it.

This command illustrates the final continuation; the saved run configuration determines the actual experiment:

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/train.py \
  --base Qwen/Qwen3-4B-Instruct-2507 \
  --revision cdbee75f17c01a7cc42f958dc650907174af0554 \
  --qlora --sets slm-distill/data/research-v3/continuation-sets \
  --adapter slm-distill/out/local-4b-general-stage-a/final \
  --out slm-distill/out/local-4b-general \
  --rank 16 --accum 8 --epochs 1 --max-length 4096 --loss-chunk-size 256 --lr 3e-5 --eval-on-epoch
```

It depends on the preserved stage-A adapter and frozen curriculum. A from-scratch run is a different experiment unless all earlier stages and inputs are reproduced. Do not overwrite completed run directories.

## Evaluate before release

The internal matched evaluation has 64 distinct episodes, balanced across domains and tasks. The separate external set contains sixteen short LoCoMo windows, used only for academic evaluation under its CC-BY-NC-4.0 license. Neither held-out set enters training or prompt tuning.

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/evaluate_research.py \
  --model mem-extractor:general-v1 --label student-general-64 \
  --rows slm-distill/data/research-v3/fresh-evaluation-64.jsonl \
  --output-folder slm-distill/data/research-v3/eval
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/grade_research.py \
  --label student-general-64 --root slm-distill/data/research-v3
```

Repeat the matched protocol for the untrained base and external set; see the protocol for exact labels and denominators. All compared models use Q8 quantization, the same decoder and 12,288-token serving context. That larger serving limit does not establish learned long-context quality. Semantic support and coverage must be reported separately from schema acceptance, with failed windows retained in denominators.

`verify_live_memory.py` checks extraction, correction history, derived observations, graph projection, source-backed recall and cited answers on a fresh namespace. It must run against the final selected worker. A prior bridge-model pass does not certify the new candidate.

## Package and publish

`export.py` merges the completed adapter and converts it to Q8_0 GGUF. Export needs substantial temporary memory/disk space. Preserve the GGUF checksum and use the supplied non-thinking chat template.

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/export.py \
  --base Qwen/Qwen3-4B-Instruct-2507 \
  --revision cdbee75f17c01a7cc42f958dc650907174af0554 \
  --adapter slm-distill/out/local-4b-general/final \
  --name mem-extractor:general-v1 --quant q8_0
```

Use `--work` to choose the temporary workspace; merging and conversion need about 13 GB before cleanup. The supervised local pipeline checks available RAM before using `/dev/shm/mem-extractor-export` instead of the default disk workspace.

After completed evaluation and live acceptance:

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/build_release_receipt.py
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/stage_release.py \
  --adapter slm-distill/out/local-4b-general/final \
  --gguf slm-distill/out/local-4b-general/mem-extractor-q8_0.gguf \
  --receipt slm-distill/data/research-v3/release-receipt.json \
  --out /path/to/empty-release-directory
slm-distill/release.sh /path/to/empty-release-directory --verify-only
```

The receipt builder refuses missing final weights, incomplete baseline/student evaluations, grading errors or failed/missing live checks. Packaging includes only explicit public artifacts and checksums. Upload is a separate action; omitting `--verify-only` uploads the verified directory to a private staging repository. Remote verification and public publication follow separately.

Once published, the model repository's README provides Ollama and direct PEFT loading instructions. The public name is `mem-extractor`; older `memex-extractor` names in experiment receipts are historical aliases.

## Historical reproduction

[`v1/README.md`](v1/README.md) preserves an older 1.7B pipeline, schema, hardware and results. Its metrics and LoCoMo-derived training setup do **not** describe the current 4B general-purpose release. The current external LoCoMo diagnostic is held out, unlike that archived experiment.

The later local 1.7B bootstrap/refinement and initial 4B coding stage are documented in the iteration notebook. Their strong synthetic scores and weak naturalistic extraction results motivated the current data, prompt and decoder changes. Preserve those failures rather than replacing them with final-run numbers.
