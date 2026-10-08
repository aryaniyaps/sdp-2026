# Benchmark runbook

This is the handoff for running the complete experiment. Execution was delegated to
the project team on 2026-10-05 after the local subscription-sharing quota was exhausted.
The implementation and native three-session Pi demonstration have been exercised;
**the full memory benchmark is unfinished and no improvement is claimed.**

## 1. Prepare your machine and model access

Use Linux or a suitable Linux environment, Docker with Compose, Node.js 22.19 or
newer, npm, Python 3, `curl`, `rg`, and Git. The supplied runtime builds a Linux Rust
binary inside Docker and runs it on the host; the supplied test script uses host
networking. Running these scripts directly on macOS or Windows is not equivalent.

Install the pinned harness and authenticate with your own provider:

```sh
npm install -g --ignore-scripts @earendil-works/pi-coding-agent@1.0.2
pi --version
pi
# Inside Pi, use /login for your supported subscription or API-key provider.
# Exit Pi after authentication.
pi --list-models
```

Never copy another person's credentials into the repository. The model aliases in
`config.json` (`openai/gpt-5.6-sol` and `openai/gpt-6.1-sol`) worked in the original
environment; do not assume a fresh Pi installation exposes them. Select models that
your own authenticated provider actually supports. Before starting a **new** run,
edit these fields in `config.json`:

| Fields | Purpose |
|---|---|
| `worker_provider`, `worker_model` | Extraction and consolidation: `ollama` and the installed model name (default `memex-extractor`) |
| `worker_thinking` | `none`: the worker is the fine-tuned model served by Ollama, which does not think |
| `reader_provider`, `reader_model`, `thinking` | Same reader for all conditions; `thinking` also applies to the judge |
| `judge_provider`, `judge_model` | Blinded category-rubric judging |
| `worker_concurrency`, `ingestion_concurrency` | Parallel independent memory banks |

Keep the chosen configuration unchanged after freezing. The reader needs sufficient
context capacity for complete histories: the largest history is 540,498 characters
in this dataset (about 135,125 tokens under the runner's character estimate).
The original reader advertises a 272K context window. An estimate is not an exact
tokenizer count; a context-limit error must be resolved with a suitable configuration
and a new run, not by quietly truncating the full-history baseline.

This is a substantial workload. It retains 23,867 complete episodes, issues 2,500
retrieval requests, and produces 3,000 reader answers and 3,000 judgments across the
planned conditions and repetitions, plus extraction/consolidation inference and
repairs. Subscription limits can interrupt it. API-key providers may bill usage;
check your provider's allowance before running. The reported Pi cost fields are
model estimates, not proof of a subscription charge.

## 2. Start and validate the service

From the repository root, make sure the worker model named in `config.json` is installed (`run-memory.sh` does that) and match the concurrency:

```sh
export EXTRACTION_MODEL='memex-extractor'
export MEMORY_WORKER_CONCURRENCY=4
./scripts/run-memory.sh
```

Use the configuration's concurrency value if you changed it. Keep this terminal
running. The script starts PostgreSQL/Neo4j, obtains local
`qwen3-embedding:0.6b` embeddings and builds the pinned Rust toolchain in Docker.
Ports are 8080 (API/UI), 55432 (PostgreSQL), 7474/7687 (Neo4j), and 11434 (Ollama).
The embedding model produces 1,024-dimensional vectors.

In another terminal:

```sh
curl -fsS http://127.0.0.1:8080/healthz
./scripts/test.sh
```

Check database and embedder readiness, worker identity, embedding model and worker
concurrency. The worker is the Ollama model named in `config.json`; the reader and judge still run through Pi. A model identity mismatch stops ingestion. The normal
runtime is `run-memory.sh`; the Compose `full` API profile uses a different Ollama
worker configuration and is not the pinned subscription experiment.

## 3. Fetch, pilot, then freeze a new full run

```sh
python3 benchmark/run.py fetch
python3 benchmark/run.py all --run friend-pilot-10 --limit 10
```

`fetch` verifies the dataset SHA-256 and pins the current upstream rubric revision.
It can update the vendored rubric; do this **before** freezing. Do not call it to
refresh an existing frozen run. The dataset is about 277 MB and is intentionally
excluded from Git.

Inspect the pilot's jobs, answers and traces. A pilot is a development run and cannot
satisfy full completion. Resolve failures before starting the full experiment:

```sh
python3 benchmark/run.py prepare --run friend-review-full
python3 benchmark/run.py all --run friend-review-full > benchmark-friend.log 2>&1
```

The full command has **no `--limit`** and uses all primary conditions and ablations.
`prepare` freezes a source archive, configuration, dataset/rubric provenance, Pi
version, question IDs and the deterministic 100-question repeat sample. Changes to
the fingerprinted code/config or question set require a new run ID. The archive is
the authoritative code snapshot when the Git checkout was dirty at freeze time.

The `all` command runs ingestion, retrieval, primary answers/judgments, both extra
repetitions, reporting and audit-packet generation. It waits for ingestion of all
questions before starting its own retrieval phase. Do not run duplicate writers for
the same run/condition/repeat. The optional independent full-history reader can run
early, but the simplest reproducible workflow is the single `all` process above.

The original resolver baseline was retired with the V1 API. Existing runs remain
historical artifacts; start a new run for the current three-condition protocol.

## 4. What is being compared

| Condition | Retrieval / context | Answer count |
|---|---|---:|
| `raw_hybrid` | Lexical/vector search over retained source chunks | 500 + 100 + 100 |
| `enhanced` | Temporal assertions, supported observations and bounded graph expansion | 500 + 100 + 100 |
| `full_history` | Complete timestamped histories, without memory retrieval | 500 + 100 + 100 |
| `no_graph` | Enhanced retrieval with graph expansion disabled | 100 |
| `no_consolidation` | Enhanced retrieval with observations excluded | 100 |

Each retrieval condition runs on all 500 questions, including both ablations.
Repetitions 1 and 2 use only the fixed 100-question sample; repeat 0 already contains
those questions. Ablation answers use that same sample. All readers are fresh,
tool-free Pi processes. Ingestion excludes questions, reference answers and evidence
labels. Repeated dataset session IDs keep their original provenance/scoring identity
but receive distinct episode idempotency keys.

The judge uses pinned upstream category rubrics with the configured subscription
model, without condition names in its prompts. These are custom experimental scores,
not official GPT-4o LongMemEval scores or a reproduction of a vendor's leaderboard.

## 5. Inspect and resume safely

```sh
curl -fsS 'http://127.0.0.1:8080/api/v2/status?namespace=longmemeval:friend-review-full:QUESTION_ID'
curl -fsS 'http://127.0.0.1:8080/api/v2/jobs?namespace=longmemeval:friend-review-full:QUESTION_ID'
```

Pending/running/failed jobs make a bank unready. Readiness with missing embeddings
also fails the benchmark. Inspect job errors and the UI traces rather than dropping
questions, replacing failed answers with defaults, or accepting degraded retrieval.
The job listing shows the latest 100 jobs; the status endpoint still counts all work.

After a terminal interruption, rerun the same command with the same run ID. Completed
artifacts are skipped, and immutable episode requests are idempotent. A live process
that has not printed recently is not automatically failed: inspect its process and
job status before starting another writer.

For subscription-quota failures on the matching existing database:

```sh
python3 scripts/resume-evaluation.py --run friend-review-full --check
# After access resets:
python3 scripts/resume-evaluation.py --run friend-review-full
```

The helper checks the freeze, Pi version, service identity, and failed-job scope.
It probes the required models before any retries, retries only quota failures in this
run, then resumes `all`. Add `--resume-demo` only to finish processing an existing local
demo with all three sessions already recorded. It stops for non-quota job failures
that need investigation. It assumes the provided Compose PostgreSQL service and
`memory_app` database. It does not switch providers or use new credentials for you.

The original `review-full-v3` partial run has 102 primary full-history answers, 27
repeat-1 answers, 17 repeat-2 answers and 90 primary full-history judgments. These are
incomplete artifacts, not an improvement result. Earlier runs containing
`retired.json` must not be used for final reporting.

**On another machine, use a new run ID and rebuild all banks.** Dataset, run artifacts,
model cache and retained evidence are not in Git. Copying only the run folder is
insufficient: `ingestion.json` can say a bank is complete while the new database has
none of its evidence. Resuming the original run requires the matching PostgreSQL
database, artifacts and exact freeze; reconstruct the Neo4j projection if necessary.

## 6. Complete the human audit and final report

After all answers and judgments finish:

```sh
python3 benchmark/run.py audit --run friend-review-full
```

`audit/packet.json` must contain 300 items: the fixed 100 questions across three primary
conditions at repeat 0. Keep `audit/key.json` with the coordinator, away from reviewers.
Two actual people independently assess the hypothesis against its question/reference
and category rubric. Record their decisions as JSON Booleans in `reviewer_1` and
`reviewer_2`; leave `adjudicated` null when they agree. For disagreements, a separate
adjudication records a Boolean in `adjudicated`. Keep reviewer identities and notes
in your review records. Do not use model-generated labels as human reviews.

Regenerating the packet preserves existing labels and rejects changed answer content.
Once human review is complete, regenerate the report:

```sh
python3 benchmark/run.py report --run friend-review-full
```

Inspect `report.json` and the raw files, not just one accuracy number. The report
contains category accuracy, abstention results, session-retrieval recall/nDCG,
packed-context recall, paired bootstrap intervals, repeat variability, operational
timing and human/judge agreement.

Final completion requires:

- 500 complete history banks and all 2,500 retrieval artifacts.
- 500 repeat-0 answers/judgments for each primary condition.
- 100 answers/judgments for each primary condition at repetitions 1 and 2.
- 100 answer/judgment artifacts for each ablation.
- 300 independently double-reviewed audit items, with every disagreement resolved.
- `report.json` has `primary_complete`, `repeats_complete` and `complete` equal to true.
- Raw artifacts and model/configuration provenance support those counts; report flags
  alone do not verify every requirement in `project-requirements.json`.

For an improvement claim, inspect the paired difference against each baseline and
its 95% interval, category results and sample coverage. Report negative or inconclusive
outcomes honestly. A favorable anecdote or partial subset does not prove improvement.

## 7. Deliverable bundle and native coding demo

Preserve the run's `manifest.json`, `source.tar.gz`, `service.json`, `ingestion.json`,
retrieval/answer/judgment files, `report.json`, audit packet/key, logs and human review
records. Do not publish credential files. The audit key is for the coordinator.

The separate actual Pi coding demonstration is reproducible with:

```sh
python3 benchmark/demo/run.py
python3 benchmark/demo/verify.py
```

Its harness/model settings are currently fixed in `demo/run.py`, so check their
availability before running on another machine. It uses a disposable repository and
three fresh sessions. The existing local demonstration passed seven unit tests and
twelve independent API checks, with the microsecond correction injected into session
three. Final-session evidence processing stopped at the subscription quota. See
[`docs/pi-demo-results.md`](../docs/pi-demo-results.md).

The demo proves native integration and remembered decisions across sessions; it has
no paired no-memory coding control. The controlled experiment above measures memory
QA performance. Use [`docs/subsystem-review.md`](../docs/subsystem-review.md) for the
four subsystem explanations and [`docs/completion-audit.md`](../docs/completion-audit.md)
for the implementation/evaluation boundary.
