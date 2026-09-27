# Evidence graph memory: implementation and review guide

This implementation keeps PostgreSQL authoritative and treats Neo4j as a replayable projection. A graph outage cannot erase retained evidence. Assertions carry source quotes, temporal intervals, confidence, and model identity; observations carry explicit supporting assertion IDs. The implementation is still undergoing live evaluation. No accuracy improvement is claimed yet.

## Run

Install Docker Compose, Node/npm, and Pi with an authenticated `openai` subscription provider. The runtime script builds Rust in a pinned container and uses local Qwen embeddings. It does not need a hosted embedding API.

```sh
./scripts/run-memory.sh
# UI: http://127.0.0.1:8080
./scripts/test.sh
# In a separate terminal, inside a repository:
pi -e /absolute/path/to/sdp-2026/integrations/pi/extension.ts
```

`PI_PROVIDER` and `PI_MODEL` select the worker; defaults are `openai` and `gpt-5.6-sol`. `MEMORY_URL`, `MEMORY_NAMESPACE`, and `MEMORY_SPOOL` configure the extension. The service binds localhost. The Compose credentials are development credentials.

## Four technical workstreams

| Workstream | Code | Technical contribution | Review demonstration |
|---|---|---|---|
| Temporal storage and graph | `knowledge_store.rs`, migrations, `graph.rs` | Namespace isolation, atomic episode ingestion, cardinality, correction intervals, transactional jobs, leases, projection revisions | Correct Python to Rust, inspect old/new validity and exact evidence; replay the graph |
| Evidence consolidation | `knowledge.rs`, `model.rs`, `worker.rs` | Schema-validated subscription inference, exact quotation checks, grounded aliases, supported observations, recursive invalidation | Derive an observation from two facts, then retract a support and show the descendant becomes stale |
| Retrieval and explanations | `v2.rs`, `ui.html` | Lexical/vector/temporal retrieval, graph expansion, authoritative validation, deterministic RRF and bounded evidence context | Compare graph and observation ablations, inspect ranks, paths, timestamps and evidence |
| Harness and evaluation | `integrations/pi`, `benchmark` | Automatic recall, observed tool outcomes, local spool, isolated benchmark banks, blinded judging, paired confidence intervals | Resume Pi across sessions; inspect captured command exit status and trace; reproduce the experiment |

These are subsystem ownership suggestions. Team members should explain and verify their own implementation contributions rather than present suggested assignments as historical work.

## Storage semantics

A single-valued slot has at most one active value. A sufficiently confident explicit correction supersedes its previous value and closes its validity interval. An unexplained competing value becomes contested. Multi-valued facts remain additive; event facts include event time in reinforcement identity. Assertions are never silently replaced by a new source. A retroactive correction can become valid before the previous belief’s inferred start; the superseded belief closes at the later of its own start and the correction date, producing an empty interval when the old belief was erroneous throughout. Recorded timestamps preserve when both beliefs entered storage.

Quotes must be exact substrings of the cited input events. Entity aliases are retained only when they occur in supporting evidence. Observation support must refer to active assertions within the same namespace. Removing or superseding a support recursively marks dependent observations stale; stale assertions are excluded from current retrieval. Consolidation creates new observations pointing to existing supports, avoiding dependency cycles.

An idempotency key identifies an immutable episode request. Reusing the key with different content fails. Durable jobs use leases and token fencing. Namespace leases allow four banks to process concurrently while preserving episode order within each bank, including after failures. Consolidation waits for queued extraction to finish. Model responses are cached separately from stored claims. Graph writes carry monotonically increasing revisions so an older projection cannot overwrite a newer assertion. Graph candidates are rechecked against PostgreSQL before use.

## V2 API

All namespaces must be explicit. Requests and responses can be inspected in the source types in `knowledge.rs` and `v2.rs`. The original V1 endpoints remain available.

| Method and path | Purpose |
|---|---|
| `POST /api/v2/retain` | Store an episode and queue extraction; supply namespace, session_id, external_id, events and metadata |
| `POST /api/v2/recall` | Retrieve attributed evidence with query, namespace, graph/observations switches, optional as_of/from/to and token budget |
| `POST /api/v2/reflect` | Produce a bounded response citing retrieved assertion IDs |
| `GET /api/v2/status?namespace=...` | Observe queued, running and failed work plus projection progress |
| `GET /api/v2/jobs?namespace=...` | Inspect processing jobs |
| `GET /api/v2/jobs/{id}?namespace=...` | Inspect one job and its result/error |
| `POST /api/v2/jobs/{id}/retry?namespace=...` | Retry a failed job |
| `GET /api/v2/graph?namespace=...` | Inspect graph entities, assertions and edges |
| `GET /api/v2/assertions/{id}?namespace=...` | Inspect assertion provenance |
| `POST /api/v2/assertions/{id}/retract?namespace=...` | Retract a fact and invalidate supported observations |
| `POST /api/v2/graph/rebuild?namespace=...` | Queue projection reconstruction |

Readiness includes failures: pending, running, or failed jobs make a namespace unready. A submission acknowledgment means evidence was retained, not that extraction has finished.

## Evaluation

The runner pins the cleaned LongMemEval-S dataset and upstream judgment rubric with checksums. It ingests complete histories through an allowlist that excludes questions, answers and evidence labels. Banks are isolated per question. Readers run in fresh temporary directories with tools, extensions, skills and memory disabled.

```sh
python3 benchmark/run.py fetch
# Development run, explicitly excluded from full completion:
python3 benchmark/run.py all --run development-10 --limit 10
# Freeze implementation first, then run all 500:
python3 benchmark/run.py all --run review-full
```

Stages are resumable: `prepare`, `ingest`, `retrieve`, `answer`, `judge`, `report`, and `audit`. A manifest rejects reuse after implementation/configuration changes. Raw hybrid, legacy, enhanced and full history are primary conditions; graph and consolidation ablations are included. The fixed 100-question sample receives two additional repetitions. Reports expose graded counts and paired bootstrap intervals. Human audit packets hide the condition; two reviewers fill Boolean labels, and disagreements require adjudication. Recreating a packet preserves existing reviews and rejects changed answers.

Do not interpret a pilot, partial run, or favorable isolated example as a benchmark result. Check `project-requirements.json` for outstanding completion requirements. Runtime throughput and subscription availability determine how long the full experiment takes.

## Research references

The design draws on [Supermemory's documented memory relationships](https://supermemory.ai/docs), [Hindsight's retain/recall/reflect architecture](https://github.com/vectorize-io/hindsight), [Graphiti's temporal graph approach](https://github.com/getzep/graphiti), and [LongMemEval](https://github.com/xiaowu0162/LongMemEval). These are references, not claims of equivalent behavior or performance. The project uses its own Rust/PostgreSQL implementation.

### How the references influenced this implementation

| Primary reference | Referenced idea | Implementation choice |
|---|---|---|
| [Supermemory architecture](https://github.com/supermemoryai/supermemory/blob/main/skills/supermemory/references/architecture.md) | Memories evolve through updates, extensions and derivations; historical versions remain available | Explicit supersedes/extends/derives edges, temporal assertions and supported observations with preserved exact sources |
| [Hindsight retain](https://hindsight.vectorize.io/developer/retain) | Retain starts asynchronous consolidation; observations track supporting facts | Durable extraction/consolidation jobs, support edges and transitive invalidation when evidence changes |
| [Hindsight recall](https://hindsight.vectorize.io/developer/retrieval) | Multiple retrieval strategies use reciprocal rank fusion and a separate context budget | PostgreSQL lexical/vector/temporal candidates, bounded Neo4j expansion, deterministic RRF and evidence packing |

These are adaptations, not copied implementations. This project uses native PostgreSQL full-text ranking, not a BM25 extension; it does not implement Hindsight's cross-encoder reranker. Its derivations require explicit support rather than unrestricted profile inference. Correctness fencing and replayable projection are implemented locally, and their benefits must be distinguished from measured answer accuracy. References were checked on 2026-10-05.
