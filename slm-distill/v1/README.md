# SLM extraction distillation

Distils the 14B extractor this project currently calls (`EXTRACTION_MODEL=qwen2.5:14b-instruct-q4_K_M`)
down to a **1.7B student** that produces the same typed-memory schema, so the extraction
stage can run on a laptop-class GPU instead of needing a 14B resident in Ollama.

The teacher is used **once, offline**, to label training data. It is never on the serving
path, so the "runs locally, no cloud dependency" property of the engine is unchanged.

```
LoCoMo sessions ──► teacher (hosted)  ──► silver corpus ──► QLoRA ──► adapter
                     once, offline         784 windows      6.4 min    139 MB
                                           3,706 memories
                                                                          │
                                    serving path ◄────────────────────────┘
                                    student only, local, no network
```

## Results

142 held-out windows from 2 conversations that share no speakers with the training split.
6 windows where the teacher extracted nothing are excluded, so nothing gets free credit
for both sides being empty (n=136).

| metric | base zero-shot | distilled | |
|---|---|---|---|
| relaxed F1 (type + subject + statement overlap) | 0.000 | **0.398** | |
| relaxed F1, type ignored | 0.000 | 0.541 | |
| entity F1 (found the right people) | 0.465 | 0.797 | |
| strict F1 (exact `subject::predicate` string) | 0.000 | 0.180 | lower bound, see below |
| windows with ≥1 genuine match | 0/136 | **73/136** | |
| JSON parse rate | 93.7% | **100%** | |

**The headline is schema compliance.** Field completeness across every emitted memory:

| field | base zero-shot | distilled |
|---|---|---|
| `statement` | 0/546 (0.0%) | 711/711 (100%) |
| `object` | 5/546 (0.9%) | 711/711 (100%) |
| `subject` | 363/546 (66.5%) | 711/711 (100%) |
| `type` | 426/546 (78.0%) | 711/711 (100%) |
| `entity_key` | 414/546 (75.8%) | 711/711 (100%) |
| `evidence` | 472/546 (86.4%) | 711/711 (100%) |

The base model never emitted a `statement` field, which is why its relaxed F1 is a true
zero rather than a rounding artefact: there was nothing to compare against. Distillation
took a model that could not produce this schema at all into one that produces it on every
memory.

Training: 2 epochs, 642 examples, **381 s**, **6.8 GB peak VRAM** on an RTX 5070 Ti
(16 GB, Blackwell `sm_120`). Adapter is 34.9M params (1.99% of 1.76B), 139 MB on disk.
`eval_loss` 0.2645 → 0.2022, monotone.

## Honest limitations

- **These are agreement scores, not accuracy.** The teacher's output is the reference. A
  student that faithfully imitates a wrong teacher scores well here. Human-verified ground
  truth is not part of this evaluation.
- **Latency currently regresses**: p50 2,868 ms (teacher) → 6,115 ms (student). The student
  emits more memories per window (5.0 vs 4.3), and this is unbatched HuggingFace `generate`
  in Python, not the llama.cpp/Ollama path the engine actually uses. Not yet a like-for-like
  comparison, and as measured it misses the "≤ ⅓ teacher latency" target.
- **Type confusion is the largest fixable error.** Relaxed F1 0.398 vs 0.541 with type
  ignored — 0.143 F1 is lost purely to mislabelling, mostly collapsing `task` (an intention)
  into `episode` (a completed event): teacher `evan::plans_to_repair` → student
  `evan::repaired`.
- **Subject misattribution** happens on multi-speaker windows: the student occasionally
  assigns one speaker's preference to the other.
- **The strict metric is not meaningful as a headline.** `entity_key` is
  `subject::predicate` where the predicate is free-form generated text, so `sam::likes` and
  `sam::loves` for the identical statement score as a total miss. Kept only as a lower bound.

## Not wired into the Rust service yet

This PR adds the pipeline and its evidence. Actually swapping the extractor over still needs:

1. merge the LoRA into the base weights and convert to GGUF,
2. `ollama create` the merged model,
3. point `EXTRACTION_MODEL` at it,
4. re-run `tests/postgres_api.rs` and compare traces.

The adapter itself (139 MB) is **not** committed — it exceeds GitHub's file limit.
Reproduce it with `./run_all.sh`.

## Reproducing

```bash
cd slm-distill
./setup.sh                     # venv, torch (PyPI, not download.pytorch.org), Qwen3-1.7B
cp .env.example .env           # add teacher credentials
python gen_labels.py           # ~16 min, regenerates data/silver.jsonl
./run_all.sh                   # train (~6 min) then evaluate
python score.py                # re-score saved generations, no inference needed
```

`data/gen_*.json` holds every raw generation from the eval run, so scoring rules can be
changed and re-applied without re-running the model.

LoCoMo (`data/locomo10.json`) is not redistributed here; `gen_labels.py` documents the
source. Only the derived labels are committed.

## Files

| file | purpose |
|---|---|
| `schema.py` | the extraction schema + system prompt, shared by teacher and student |
| `gen_labels.py` | teacher → silver corpus, resumable, schema-validated |
| `train_lora.py` | QLoRA finetune; loss masked to the JSON target only |
| `eval.py` | base zero-shot vs distilled on the same held-out windows |
| `score.py` | four matching rules over saved generations |
| `setup.sh` / `run_all.sh` | resilient setup and the full pipeline |
