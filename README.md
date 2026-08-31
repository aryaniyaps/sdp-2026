# Memory Engine — Observable Temporal Memory in Rust

An offline-capable technical prototype that turns interaction text into typed, versioned memories while retaining the raw evidence behind every fact. It uses Rust, Axum, PostgreSQL full-text search, pgvector, and local Ollama models—and makes every internal decision inspectable in the demo.

This is a production-shaped vertical slice, not a mock UI. Ingestion, model calls, database transactions, provenance, correction chains, hybrid retrieval, observability, and the review interface run through the same application code.

Last reviewed: 31 August 2026.

## What the demo proves

The scenario starts with “Aryan prefers Python for programming” and later receives “Aryan now prefers Rust for programming.” The engine does not overwrite Python. It closes Python's validity interval, creates an active Rust version, adds a `supersedes` edge, and retains both original source events. Default retrieval returns only active Rust; history still proves where Python came from.

The **Operation observatory** exposes the pipeline behind that result:

- immutable event and chunk IDs;
- extractor provider, model, schema mode, output, and latency;
- embedding model and verified 1,024-dimensional vector size;
- normalized `subject::predicate` identity and normalized value;
- transactional `created`, `reinforced`, or `superseded` decisions;
- lexical and semantic candidates before fusion;
- lexical rank, semantic rank, and exact RRF score;
- token budget, included versions, final context, and timing;
- degraded-mode reasons when embeddings are unavailable.

## Architecture

```text
Interaction text
      │
      ▼
Axum API ──► raw_events + chunks (immutable evidence)
      │
      ├──► Ollama extractor (typed JSON, temperature 0, repair retry)
      ├──► canonical subject::predicate
      ├──► Ollama embedder ──► vector(1024)
      ▼
PostgreSQL transaction
  memories ──► memory_versions ──► supersedes relation
                    └────────────► source provenance

Search query
      ├──► PostgreSQL FTS / GIN (top 20)
      ├──► pgvector HNSW cosine search (top 20)
      └──► reciprocal-rank fusion, k=60
                    ▼
             token-budget packer
                    ▼
          context + ranks + provenance

Every operation ──► operations + operation_steps
                 ├──► trace explorer UI
                 ├──► correlated JSON logs
                 └──► /metrics counters
```

## Technology stack

| Layer | Technology | Purpose |
|---|---|---|
| Service | Rust 1.96, Tokio, Axum | Async API and embedded UI |
| Persistence | PostgreSQL 17, SQLx 0.9 | Transactions, versions, provenance |
| Retrieval | PostgreSQL FTS + pgvector 0.4 | Lexical and semantic candidates |
| Extraction | Ollama `qwen2.5:14b-instruct-q4_K_M` | Local typed-memory extraction |
| Embeddings | Ollama `qwen3-embedding:0.6b` | Local 1,024-dimensional embeddings |
| API docs | Utoipa + Swagger UI | Generated executable API contract |
| Observability | tracing, PostgreSQL traces, Prometheus text | Logs, explanations, counters |
| Packaging | Docker Compose | Repeatable database and API runtime |

## Prerequisites

Install Docker Engine with Compose, Ollama, `curl`, and a POSIX shell. Allow enough disk space for both Ollama models and Docker images; the 14B extractor is the largest download.

```bash
docker --version
docker compose version
ollama --version
curl -fsS http://127.0.0.1:11434/api/version
```

You do **not** need Rust on the host. Compilation and tests use the pinned `rust:1.96-bookworm` container.

## Run the demo: exact steps

### 1. Clone and enter the repository

```bash
git clone https://github.com/aryaniyaps/sdp-2026.git
cd sdp-2026
```

### 2. Start Ollama

If it is not already running, start it and keep this terminal open:

```bash
ollama serve
```

### 3. Launch the complete stack

In a second terminal:

```bash
./scripts/review.sh
```

The launcher checks Docker and Ollama, starts PostgreSQL, waits for health, downloads missing models, pre-warms them, builds the pinned Rust release container, applies migrations on API startup, and prints every review URL. The first launch can take several minutes because it downloads the model and compiles dependencies; later runs reuse local caches.

### 4. Confirm readiness

```bash
curl -s http://127.0.0.1:8080/healthz | jq
```

Expected response:

```json
{"database":true,"extractor":true,"embedder":true,"degraded_mode":false}
```

| Surface | URL or command |
|---|---|
| Demo and operation observatory | <http://127.0.0.1:8080> |
| Swagger UI | <http://127.0.0.1:8080/swagger-ui/> |
| Health | <http://127.0.0.1:8080/healthz> |
| Prometheus metrics | <http://127.0.0.1:8080/metrics> |
| Structured logs | `docker compose logs -f api` |

PostgreSQL is bound only to `127.0.0.1:55432`; the API is bound only to `127.0.0.1:8080`.

## Five-minute review sequence

1. Open the UI and confirm DB, extraction, and embedding readiness.
2. Keep `Aryan prefers Python for programming.` and click **Live extraction**. Use **Deterministic fixture** only if live inference is too slow.
3. In **Operation observatory**, walk through evidence persistence, extraction, embedding, version resolution, and finalization. Point out real IDs, model names, dimensions, output, and per-stage milliseconds.
4. Search the default question. Show both candidate sets, RRF formula `1 / (60 + rank)`, fused scores, and token packing.
5. Click **Load Rust correction**, ingest it, and inspect the `superseded` transaction decision.
6. Search again. Context contains Rust only while the ledger keeps superseded Python with its quote and timestamps.
7. Open `/metrics` or follow API logs to show counters and structured runtime events.

The deterministic fixture is deliberately labelled and requires `DEMO_MODE=true`. It bypasses model extraction only. Evidence storage, embedding, transactional resolution, trace recording, and retrieval still use the real path. Live extraction never silently fabricates fallback facts.

## Observability surfaces

### Durable operation traces

Every completed, degraded, or model/versioning-failed operation receives a UUID returned as `trace_id`. Traces survive refreshes and API restarts because they are stored in PostgreSQL.

```bash
curl -s 'http://127.0.0.1:8080/api/v1/traces?namespace=review&limit=20' | jq
curl -s 'http://127.0.0.1:8080/api/v1/traces/TRACE_UUID' | jq
```

`operations` stores request/result summaries, correlation IDs, status, and total latency. `operation_steps` stores ordered stages with duration and structured evidence. Trace requests record interaction character counts rather than copying raw text; the evidence tables remain authoritative.

### Structured logs and metrics

```bash
docker compose logs -f api
curl -s http://127.0.0.1:8080/metrics
```

Logs are JSON by default and include HTTP method, URI, response status, latency, and application operation IDs. Set `LOG_FORMAT=pretty` for human-readable local logs. Metrics include operation totals, failures, degraded operations, extracted memories, version decisions, and cumulative latency. Counters reset with the process; database traces do not.

### Direct database inspection

```bash
docker compose exec postgres psql -U memory -d memory
```

```sql
SELECT id, operation_type, status, duration_ms, started_at
FROM operations ORDER BY started_at DESC;

SELECT ordinal, stage, status, duration_ms, details
FROM operation_steps
WHERE operation_id = 'TRACE_UUID' ORDER BY ordinal;

SELECT m.canonical_key, mv.version, mv.value, mv.status,
       mv.valid_from, mv.valid_to
FROM memories m JOIN memory_versions mv ON mv.memory_id = m.id
ORDER BY m.canonical_key, mv.version;
```

## API quick start

```bash
SESSION_ID=$(curl -s -X POST http://127.0.0.1:8080/api/v1/sessions \
  -H 'content-type: application/json' \
  -d '{"namespace":"review","external_id":"cli-demo"}' | jq -r .id)

curl -s -X POST http://127.0.0.1:8080/api/v1/events \
  -H 'content-type: application/json' \
  -d "{\"session_id\":\"$SESSION_ID\",\"role\":\"user\",\"content\":\"Aryan prefers Python for programming.\"}" | jq

curl -s -X POST http://127.0.0.1:8080/api/v1/search \
  -H 'content-type: application/json' \
  -d '{"namespace":"review","query":"What language does Aryan like to code in?","top_k":5,"max_tokens":128}' | jq

curl -s 'http://127.0.0.1:8080/api/v1/memories?namespace=review&include_history=true' | jq
```

## Algorithms and invariants

Memory identity is normalized `subject::predicate`; values are independently normalized for duplicate detection.

- No active version: create version 1.
- Same normalized value: add provenance without duplicating the version.
- Changed value: close the old validity interval, create the next version, and insert a `supersedes` edge atomically.
- Older update timestamp: reject with HTTP 409 instead of corrupting temporal validity.

Search takes 20 lexical and semantic candidates independently. RRF computes `Σ 1 / (60 + rank)`. Context takes at most `top_k` complete statements and estimates tokens as `ceil(Unicode characters / 4)`. When embedding fails, the trace marks semantic search skipped and returns lexical results with `degraded_mode=true`.

## Run the complete test suite

```bash
./scripts/test.sh
```

This runs formatting, Clippy with warnings denied, unit tests, migrations, and PostgreSQL/API integration tests in Docker. It covers normalization, extraction validation, RRF, token packing, vector dimensions, one-active-version invariants, supersession, provenance, namespace isolation, invalid input, provider failure, out-of-order conflict, hybrid retrieval, degraded retrieval, and observability persistence.

## Stop, restart, and reset

```bash
# Stop without deleting data
docker compose --profile full down

# Restart
./scripts/review.sh

# Reset demo evidence and traces
curl -X POST http://127.0.0.1:8080/api/v1/demo/reset

# Permanently delete the local PostgreSQL volume
docker compose --profile full down -v
```

## Troubleshooting

### Model unavailable

```bash
curl -fsS http://127.0.0.1:11434/api/version
ollama list
ollama pull qwen2.5:14b-instruct-q4_K_M
ollama pull qwen3-embedding:0.6b
```

### Port already in use

```bash
ss -ltnp | grep -E ':(8080|55432)'
docker compose --profile full ps
```

### API container exits

```bash
docker compose logs --tail=200 api
docker compose logs --tail=100 postgres
```

### Slow first extraction

Model loading depends on hardware. The trace isolates extractor latency from database work. Use the clearly labelled deterministic fixture to keep a live review moving.

### Degraded search

Inspect `embed_query` in the selected trace. It contains the model and error; `semantic_retrieval` will be `skipped`, while lexical retrieval remains operational.

## Repository map

| Path | Responsibility |
|---|---|
| `src/api.rs` | HTTP orchestration and stage instrumentation |
| `src/observability.rs` | Trace model/builder and process metrics |
| `src/providers.rs` | Ollama extraction and embedding adapters |
| `src/store.rs` | Persistence, version transactions, retrieval, traces |
| `src/search.rs` | RRF and token-budget algorithms |
| `src/domain.rs` | Memory and provenance domain types |
| `src/ui.html` | Embedded demo and trace observatory |
| `migrations/` | PostgreSQL/pgvector schema |
| `tests/postgres_api.rs` | Database and API integration evidence |
| `scripts/review.sh` | One-command launcher |
| `scripts/test.sh` | Containerized test suite |

## Review boundary

This increment remains a tracer bullet. Background workers, consolidation, retention/expiry, reranking, entity graphs, authentication, deletion workflows, backups, distributed trace export, load tests, and LongMemEval/LoCoMo evaluation remain future work. The observability layer demonstrates internal mechanics without claiming production operational completeness.
