# Evidence Graph Memory Engine

A Rust memory service for programming assistants and long conversations. It retains immutable evidence, extracts typed temporal assertions, derives supported observations, and retrieves attributed context through lexical, vector, temporal and graph search.

**Benchmark execution has been handed off to the project team. No accuracy improvement is claimed yet.** The original fork was merged before this enhancement. The implementation, live pilot artifacts and pending completion requirements are distinct from a completed benchmark. See the [complete benchmark runbook](benchmark/README.md).

## Run

Requirements: Docker Compose, Node/npm, Python 3, and Pi with an authenticated provider. Local embeddings use Ollama `qwen3-embedding:0.6b`; the runtime script downloads the model if needed. Rust is built with host cargo when it is the pinned toolchain, otherwise in a pinned container.

```sh
./scripts/run-memory.sh
# Observatory: http://127.0.0.1:8080
# Swagger:     http://127.0.0.1:8080/swagger-ui/
```

The extraction worker runs through Pi. Without `PI_PROVIDER` and `PI_MODEL` it uses Pi's own default model; set both to choose another (the benchmark runbook pins its own). `scripts/check-worker.py` runs first and stops with the reason if the provider is not authenticated, the model is unknown, or a small test request fails, so a misconfigured worker never starts silently. Defaults are PostgreSQL on port 55432, Neo4j HTTP on 7474, and Ollama on 11434. Override `DATABASE_URL`, `NEO4J_URI`, `OLLAMA_URL`, or `MEMORY_WORKER_CONCURRENCY` as needed. The application uses a separate `memory_app` database; tests use `memory_test`. Compose credentials are for local development.

## What changed

- **Evidence and time:** immutable, idempotent episodes; attributed exact quotes; single, multiple and event cardinalities; validity, event and recorded timestamps; grounded entity aliases.
- **Correction and consolidation:** reinforcement, competing assertions, explicit supersession, supported observations, and recursive invalidation when a support changes or is retracted.
- **Reliable graph:** PostgreSQL remains authoritative. Durable jobs, leases, heartbeats, retry fencing and projection revisions support recovery. Four inference workers process separate banks; a namespace lease and chronological queue serialize each bank, including retry backoff. A namespace reset fence prevents old graph writes from restoring cleared assertions. The original API writes through a transactional compatibility adapter.
- **Retrieval:** parallel lexical/vector candidates, temporal planning, bounded graph expansion, authoritative candidate validation, deterministic reciprocal-rank fusion, raw evidence fallback, token-bounded packing and cited reflection.
- **Programming harness:** a native Pi extension recalls prior evidence, captures messages and actual tool exit status, excludes injected context from retention, and queues offline evidence durably.
- **Evaluation:** a resumable LongMemEval-S runner with isolated question banks, full-history ingestion, baselines, ablations, repeated readers, blinded judging, paired confidence intervals and two-reviewer audit packets.

## Use with Pi

Try it in one session:

```sh
pi -e /absolute/path/to/sdp-2026/integrations/pi/extension.ts
```

Or load it in every session, in any directory:

```sh
pi install /absolute/path/to/sdp-2026/integrations/pi
```

Installing sends the content of every Pi session, including tool output, to the memory service, so do it only for a service you trust. `pi remove` undoes it.

All sessions share one memory per user (namespace `user:<login name>`), whatever directory they run in. Set `MEMORY_NAMESPACE` to keep a project's memory separate, for example from a direnv file. `MEMORY_URL` and `MEMORY_SPOOL` configure the endpoint and the local spool, which defaults to a directory derived from the namespace so any session can deliver evidence another directory queued.

Automatic recall runs before the agent starts. It skips the server's LLM date planner so it stays fast, and it also searches retained source text and ranks it with the extracted assertions, so a missed or misread extraction is less likely to hide something you just said. Evidence is queued locally before submission, and each turn retains only what is new. The extension supplies `memory_recall`, `memory_remember`, and `/memory-status`.

Retention returns as soon as the evidence is durable. Its text is searchable immediately (lexical and, once the embedder has run, semantic), while extracted facts, corrections and observations appear after the worker finishes. If extraction jobs fail, recall says so in its `degraded_reasons` and `/memory-status` shows the failed jobs.

The worker disables tools, extensions and skills during inference to avoid recursively ingesting itself. Pi reader calls in the benchmark also disable memory and tools.

## API example

```sh
curl http://127.0.0.1:8080/api/v2/retain \
  -H 'content-type: application/json' \
  -d '{"namespace":"example","session_id":"one","external_id":"event-1","events":[{"role":"user","content":"Ada prefers Python for programming.","occurred_at":"2026-01-01T12:00:00Z"}]}'

curl 'http://127.0.0.1:8080/api/v2/status?namespace=example'

curl http://127.0.0.1:8080/api/v2/recall \
  -H 'content-type: application/json' \
  -d '{"namespace":"example","query":"Which language does Ada prefer?","max_tokens":2048}'
```

Retention acknowledges durable evidence; extraction is asynchronous. Inspect jobs and wait for readiness before an evaluation. Status exposes failures, embedding gaps and processing durations. Swagger documents both API generations.

## Verify and evaluate

```sh
./scripts/test.sh
# Live check through real Pi sessions (bills your provider; needs the service running):
python3 scripts/pi-rpc-e2e.py
python3 benchmark/run.py fetch
python3 benchmark/run.py all --run development-10 --limit 10
# After freezing the implementation:
python3 benchmark/run.py all --run review-full
# Separate actual three-session coding demonstration:
python3 benchmark/demo/run.py
```

The full experiment uses all 500 cleaned LongMemEval-S questions and complete histories. Raw hybrid, original resolver, enhanced memory and full history are primary conditions. Two extra repetitions use a fixed 100-question sample; graph and consolidation ablations retrieve all 500 and answer the sample. The reader and judge use subscription models, so these are not official GPT-4o benchmark scores. Human review is required before reporting the experiment as complete.

Stages and artifacts are resumable under `benchmark/runs/`. A manifest pins data, model settings, upstream rubric and implementation fingerprint. Reusing a run after implementation changes is rejected. Development runs cannot satisfy full completion. Do not infer improvement from isolated demonstrations or partially graded questions.

See [the implementation and four-workstream review guide](docs/evidence-memory.md), [subsystem technical explanations](docs/subsystem-review.md), and [completion requirements](project-requirements.json). The earlier prototype review notes remain in `docs/`.

## Research references

The design references [Supermemory](https://supermemory.ai/docs), [Hindsight](https://github.com/vectorize-io/hindsight), [Graphiti](https://github.com/getzep/graphiti), [Pi](https://github.com/earendil-works/pi), and [LongMemEval](https://github.com/xiaowu0162/LongMemEval). This is an independent implementation; it does not claim equivalent functionality or performance.
