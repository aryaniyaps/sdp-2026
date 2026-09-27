# Evidence Graph Memory Engine

A Rust memory service for programming assistants and long conversations. It retains immutable evidence, extracts typed temporal assertions, derives supported observations, and retrieves attributed context through lexical, vector, temporal and graph search.

**Evaluation is in progress. No accuracy improvement is claimed yet.** The original fork was merged before this enhancement. The implementation, live pilot artifacts and pending completion requirements are distinct from a completed benchmark.

## Run

Requirements: Docker Compose, Node/npm, Python 3, and Pi with an authenticated subscription provider. Local embeddings use Ollama `qwen3-embedding:0.6b`; the runtime script downloads the model if needed. Rust compilation runs in a pinned container.

```sh
./scripts/run-memory.sh
# Observatory: http://127.0.0.1:8080
# Swagger:     http://127.0.0.1:8080/swagger-ui/
```

The defaults are Pi `openai/gpt-5.6-sol`, PostgreSQL on port 55432, Neo4j HTTP on 7474, and Ollama on 11434. Override `PI_PROVIDER`, `PI_MODEL`, `DATABASE_URL`, `NEO4J_URI`, `OLLAMA_URL`, or `MEMORY_WORKER_CONCURRENCY` as needed. The application uses a separate `memory_app` database; tests use `memory_test`. Compose credentials are for local development.

## What changed

- **Evidence and time:** immutable, idempotent episodes; attributed exact quotes; single, multiple and event cardinalities; validity, event and recorded timestamps; grounded entity aliases.
- **Correction and consolidation:** reinforcement, competing assertions, explicit supersession, supported observations, and recursive invalidation when a support changes or is retracted.
- **Reliable graph:** PostgreSQL remains authoritative. Durable jobs, leases, heartbeats, retry fencing and projection revisions support recovery. Four inference workers process separate banks; a namespace lease and chronological queue serialize each bank, including retry backoff. A namespace reset fence prevents old graph writes from restoring cleared assertions. The original API writes through a transactional compatibility adapter.
- **Retrieval:** parallel lexical/vector candidates, temporal planning, bounded graph expansion, authoritative candidate validation, deterministic reciprocal-rank fusion, raw evidence fallback, token-bounded packing and cited reflection.
- **Programming harness:** a native Pi extension recalls prior evidence, captures messages and actual tool exit status, excludes injected context from retention, and queues offline evidence durably.
- **Evaluation:** a resumable LongMemEval-S runner with isolated question banks, full-history ingestion, baselines, ablations, repeated readers, blinded judging, paired confidence intervals and two-reviewer audit packets.

## Use with Pi

In a repository where you want persistent memory:

```sh
pi -e /absolute/path/to/sdp-2026/integrations/pi/extension.ts
```

Automatic recall runs before the agent starts. Evidence is queued locally before submission. The extension supplies `memory_recall`, `memory_remember`, and `/memory-status`. Repository namespaces are derived from the repository root unless `MEMORY_NAMESPACE` is set. `MEMORY_URL` and `MEMORY_SPOOL` configure the endpoint and local spool.

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
