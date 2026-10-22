# Memory benchmark

This directory measures whether the memory engine helps an agent answer questions about long conversations, and
records enough to let anyone check the numbers. The results are in `Benchmark_Report.ipynb` (an executed copy
is `Benchmark_Report.html`). Every number in it is recomputed from the stored run records.

## Result of the test run

`runs/locomo-final` (commit `c45e938`, worker `mem-extractor:general-v2`), all 1,540 questions, Jev:

| condition | accuracy | context tokens |
|---|---|---|
| `enhanced` | 83.1% | 6,790 |
| `raw_hybrid` | 80.0% | 4,473 |
| `recent_window` | 43.0% | 8,168 |
| `full_context` | 89.0% | 22,400 |
| `no_memory` | 0.3% | 7 |

Every pre-registered criterion passed, on all ten conversations and on the eight held out. GPT-4o-mini gives the
same numbers within one point (agreement 96.7%, kappa 0.93). `verify` found no problems in 11,000 answers and 22,000
judgments. The report chapter's tables are generated from these records by `export_tex.py`.

## What is measured

**LoCoMo** [Maharana et al., ACL 2024] has ten long two-person conversations (272 sessions, about 16k tokens
each) and 1,540 scored questions in four categories: single-hop, multi-hop, temporal and open-domain. Category 5
(adversarial) has no gold answer and is left out, as in the Mem0 evaluation. Each conversation is retained into the
engine as its own memory bank, session by session in date order. Each question is then answered by the same reader
model under each condition:

| condition | what the reader is given |
|---|---|
| `enhanced` | the engine's recall: extracted assertions with source quotes, inferences, graph neighbours and temporal planning, packed into 8,192 tokens |
| `raw_hybrid` | the engine's lexical and vector search over raw source chunks only: plain retrieval-augmented generation |
| `recent_window` | the newest 8,192 tokens of the conversation: what an agent without memory keeps when history overflows |
| `full_context` | the whole conversation: an upper reference, and the most expensive |
| `no_memory` | the question only |
| `no_graph`, `no_consolidation` | `enhanced` without one module, on a 300-question sample stratified by category |

The reader is Claude Haiku 5.5 through the Pi coding agent (`claude-bridge` provider, a Claude subscription). The
reader prompt is Mem0's single-store answer prompt (`vendor/mem0/prompts.py`, `ANSWER_PROMPT_ZEP`) with one line
added that asks for the final answer as JSON. Two judges grade every answer against the gold answer with Mem0's
LoCoMo rubric (`vendor/mem0/llm_judge.py`, `ACCURACY_PROMPT`), and never see the condition:

- **Jev** (TypeSafe), a decision model, is the primary judge. It returns the probability that the answer is
  correct under the rubric; 0.5 is the threshold. TypeSafe serves only the `jev-latest` alias, so each judgment
  stores the model version the API reports.
- **gpt-4o-mini**, called exactly as Mem0 called it (one message, JSON output, temperature 0), so the numbers line up
  with published LoCoMo tables.

Mem0's token F1 and BLEU-1 are computed without any model. Retrieval is scored against LoCoMo's evidence labels
(which session and dialogue turns answer each question), and every source quote a recall returns is checked
against the dataset turn it cites.

**Property suite** (`properties.py`): 40 scripted conversations, generated from templates with a fixed seed, check
the behaviours the report claims on the engine's stored state, with no judge: a knowledge update keeps the old value
as history and `as_of` recall returns it; a relative day ("yesterday") is resolved to a date; reflect abstains
without evidence; a relative's move does not overwrite the user's own city; an API key is never stored. It is a
functional test of the engine, not an independent benchmark.

**Statistics.** Differences against `enhanced` are paired bootstrap intervals (2,000 resamples) over questions, and
over whole conversations (cluster bootstrap), because questions about one conversation share a memory bank. The
success criteria use the cluster interval. Repeats 1 and 2 re-ask the reader on the 300-question sample to measure
run-to-run noise. The two judges' agreement is reported as Cohen's kappa.

**Pre-registration.** The conditions, thresholds (`criteria` in `config.json`) and protocol were committed before
the test run; `git log -p benchmark/config.json` shows the order. The test run's manifest records the commit and a
fingerprint of all code and configuration, and a test run refuses to continue if either changes.

**Dev and test.** Faults were diagnosed only on the first two conversations (conv-26 and conv-30, the dev split).
`iterations.json` records each change, why it was made, and the dev numbers before and after. The test run uses
all ten conversations with the final version; the report also gives the eight conversations never used for
diagnosis on their own (`held_out`).

## Reproducing

Requirements: Linux, Docker with Compose, Rust (the version in `rust-toolchain.toml`), Node.js 22 or newer, Python
3, `uv`, `curl`, Ollama with the GPU, and these accounts:

- a Claude subscription logged in to Pi (`npm install -g @earendil-works/pi-coding-agent@1.0.2`, then
  `pi install npm:pi-claude-bridge`, then log in to Claude Code), for the reader;
- `TYPESAFE_API_KEY` and `OPENAI_API_KEY` in the environment or in the repository's untracked `.env`, for the judges.

```sh
scripts/fetch-slm.sh                          # the fine-tuned worker, checked against its SHA-256
ollama pull qwen3-embedding:0.6b               # the embedder
python3 benchmark/run.py fetch                 # LoCoMo at a pinned commit, checked against its SHA-256
benchmark/engine.sh &                          # engine on 127.0.0.1:8090 with its own Postgres and Neo4j
python3 -m unittest discover -s benchmark      # protocol tests
benchmark/overnight.sh locomo-final             # every stage, unattended; progress in runs/locomo-final.status
```

`overnight.sh` keeps the machine awake, restarts the engine if its health check fails, and retries each stage.
Every stage writes one file per unit of work under `runs/<run>/` and skips what exists, so any stage can be
re-run after an interruption. Rate limits pause every worker of that provider (honouring `Retry-After`, otherwise
backing off up to 15 minutes) and are retried for up to six hours. On the laptop used here (RTX 4070, 8 GB) the test run
took 7 h 45 min: 4 h 13 min building memory, then answering and judging.

Single stages: `python3 benchmark/run.py {ingest,recall,answer,judge,report,verify} --split test --run locomo-final`,
with `--conditions`, `--judges` and `--repeat N` to narrow them.

## Checking the results

```sh
python3 benchmark/run.py verify --split test --run locomo-final            # offline consistency check
python3 benchmark/run.py verify --split test --run locomo-final --live 20  # re-asks the models on 20 items
cd benchmark && uv run python build_notebook.py --execute                 # rebuilds the report notebook
```

`verify` rebuilds each stored reader prompt from the stored recall (or the pinned dataset) and checks its hash,
checks that each stored answer occurs in the stored model output, and rebuilds each judge rubric from the gold answer
and the stored answer. Changing any stored answer makes it fail. With `--live N` it also re-runs recall on N random
items against the engine and re-asks the reader and both judges, and reports how often they agree with the stored
records. The notebook's spot-check section prints random items in full so they can be graded by hand.

## Layout

| path | content |
|---|---|
| `config.json` | dataset pin, engine settings, reader, judges, conditions, criteria |
| `run.py` | the LoCoMo runner (standard library only) |
| `properties.py` | the deterministic property suite |
| `overnight.sh`, `engine.sh`, `docker-compose.bench.yml` | unattended run, engine launcher, the benchmark's own stores |
| `iterations.json` | every change made because of a dev finding |
| `vendor/mem0/` | Mem0's evaluation prompts and metrics at a pinned commit, with SHA-256 |
| `build_notebook.py`, `pyproject.toml`, `uv.lock` | the report notebook and its pinned environment |
| `export_tex.py` | the LaTeX tables of the report's results chapter, from `runs/<run>/report.json` |
| `runs/<run>/` | `manifest.json`, `source.tar.gz` (frozen code), `ingestion/`, `recall/`, `answers/`, `judgments/`, `repeats/`, `report.json`, `items.jsonl`, `verification.json` |
| `demo/` | the separate three-session Pi coding demonstration |

LoCoMo is licensed CC BY-NC 4.0; the run records quote it and are shared for non-commercial academic use.
