# Evidence graph memory: implementation and review guide

This implementation keeps PostgreSQL authoritative and treats Neo4j as a replayable projection. A graph outage cannot erase retained evidence. Assertions carry source quotes, temporal intervals, confidence, and model identity; observations carry explicit supporting assertion IDs. The implementation is still undergoing live evaluation. No accuracy improvement is claimed yet.

## Run

Install Docker Compose, Node/npm, and Pi with an authenticated provider. The runtime script builds Rust with host cargo when it is the pinned toolchain (otherwise in a pinned container) and uses local Qwen embeddings. It does not need a hosted embedding API.

```sh
./scripts/run-memory.sh
# UI: http://127.0.0.1:8080
./scripts/test.sh
# In a separate terminal, in any directory:
pi -e /absolute/path/to/sdp-2026/integrations/pi/extension.ts
```

The worker is the fine-tuned student model served by Ollama (`EXTRACTION_MODEL`, default `mem-extractor`, at `EXTRACTION_OLLAMA_URL`); Pi is only the coding agent that produces the evidence. `MEMORY_URL`, `MEMORY_NAMESPACE`, and `MEMORY_SPOOL` configure the extension; without `MEMORY_NAMESPACE` every directory shares the namespace `user:<login name>`. The service binds localhost. The Compose credentials are development credentials.

In Pi, `/memory-namespace [name]` changes the active namespace for the current extension session; with no name it prompts for one. Retention cursors are scoped to each namespace. `/memory-clear [name]` prompts you to type the target namespace exactly, clears its server data, and removes its pending local evidence batches. `/memory-status` reports processing for the active namespace.

### The worker model

Extraction, consolidation and reflection prompts go to `EXTRACTION_OLLAMA_URL` (default `OLLAMA_URL`, `/api/generate`) with `EXTRACTION_MODEL`, default `mem-extractor`. Every call sends the fixed system message in `src/worker/worker_system.txt`. The prompts are template files (`src/worker/extract_prompt.txt`, `src/worker/consolidate_prompt.txt`, `src/v2/reflect_prompt.txt`) shared with the training code in `slm-distill`, so the model is served exactly the text it was trained on. A start-up probe logs an error when the model is not installed (`scripts/fetch-slm.sh`). Setting `MEMORY_MODEL_PROVIDER` to anything but `ollama` stops the service.

An episode is read in windows: each event is shortened for the prompt (the start and end of long output, tool metadata reduced to the tool, exit code and short inputs), and consecutive events are grouped until a window holds about 2,800 estimated tokens. Each window is a separate model call with its own indices; claims are checked against the full, unshortened events and their indices are shifted back to the episode before they are stored. A fact corrected in a later window of the same episode relies on the store's correction handling, because the existing-facts snapshot is taken once per episode.

| Variable | Default | Meaning |
|---|---|---|
| `OLLAMA_NUM_CTX` | 16384 | Context window in tokens, sent as `num_ctx` on every generate call. |
| `OLLAMA_NUM_PREDICT` | 4096 | Cap on generated tokens, sent as `num_predict`, so a runaway answer cannot fill the context. |

Both must be positive integers and `OLLAMA_NUM_PREDICT` must be smaller than `OLLAMA_NUM_CTX`. An unusable value stops the service at startup with a message naming the variable. The values are always sent together because Ollama reloads the model whenever a request asks for a different `num_ctx`; the readiness probe behind `/healthz` sends the same one. `/healthz` reports `worker_num_ctx` and `worker_num_predict` for a local model and `null` for Pi.

Why this matters: without `num_ctx` Ollama runs the model with a 4096 token window (the model itself supports 32768) and, when a prompt is longer, keeps its first tokens and drops the middle without returning an error. Before windowing, the extract prompt was 2163 to 16658 tokens and 15 of 18 real batches were cut that way, which removed the instructions, the output schema and the quote rules. The model then answered with the wrong shape, for example `{"events":[...]}` instead of `{"claims":[...]}`.

GPU memory for the 14B q4 model, measured on a 16 GB card shared with the embedder: about 10.6 GB at 4096 tokens, about 12.9 GB at 16384 and about 13.7 GB at 20480. Raise `OLLAMA_NUM_CTX` only as far as the card allows; above the card's memory Ollama spills layers to the CPU and slows down sharply.

A prompt that cannot fit now fails loudly instead of being cut:

- Before any request, the prompt is estimated at one token per three characters (each digit counts as a token, and ten percent is added, because UUIDs and timestamps in the existing facts cost about 1.9 characters per token). The estimate is deliberately high: ordinary text is over-estimated by about a third to 60 percent, so a prompt can use only about 60 to 75 percent of the nominal window before it is refused, and raising `OLLAMA_NUM_CTX` is the remedy. If the estimate plus `OLLAMA_NUM_PREDICT` exceeds `OLLAMA_NUM_CTX`, the call fails with an error that names the estimate, the context and the variable to raise, and no request is sent.
- After each response, the counters Ollama returns are checked. A `prompt_eval_count` within 16 tokens of `OLLAMA_NUM_CTX` fails with "prompt was truncated by Ollama". A response that stops at the output cap (`done_reason` of `length`) or has no `prompt_eval_count` fails too. A rejected answer is never written to the model cache.
- The model cache is keyed on the context size as well, so an answer produced under another window, including one cut under the old 4096 default, is never replayed.

The existing-facts snapshot in the extract prompt (up to 80 facts) is the part that grows with a namespace, so it is the part that gives way. For a local model only, facts are dropped until the prompt fits with room for the answer and for a repair round (up to 1536 tokens). Facts whose subject or value appears as whole words in the text of the batch's events are kept first, strongest match first (subject and value, then subject, then value; a text of fewer than three characters, such as `go` or `5`, counts for less), so a correction of such a fact still finds the fact it replaces; among the rest the newest are kept. A correction that names its target only by a pronoun cannot be matched by text, so that fact is kept only if it is recent enough to fit. The number of dropped facts is logged as a warning and recorded as `existing_facts_dropped` in the job result. If the batch alone is too large, no facts are kept and the preflight above then refuses the prompt; raise `OLLAMA_NUM_CTX` or send smaller batches. Pi prompts are never shrunk and are byte for byte what they were before.

The tests in `tests/ollama_context.rs` use a fake Ollama server. Three more run against a real Ollama only when `OLLAMA_LIVE_TEST=1` is set (`OLLAMA_URL` and `EXTRACTION_MODEL` override the defaults); with it set they fail if Ollama or the model is unreachable. They request a 16384 token context, so they make Ollama reload the shared model at that size, which interrupts a service that runs it at another `OLLAMA_NUM_CTX` until its next call reloads it:

```sh
OLLAMA_LIVE_TEST=1 cargo test --locked --test ollama_context live_ -- --nocapture
```

## Four technical workstreams

| Workstream | Code | Technical contribution | Review demonstration |
|---|---|---|---|
| Temporal storage and graph | `src/knowledge_store/`, migrations, `graph.rs` | Namespace isolation, atomic episode ingestion, cardinality, correction intervals, transactional jobs, leases, projection revisions | Correct Python to Rust, inspect old/new validity and exact evidence; replay the graph |
| Evidence consolidation | `knowledge.rs`, `model.rs`, `src/worker/` | Schema-validated subscription inference, exact quotation checks, grounded aliases, supported observations, recursive invalidation | Derive an observation from two facts, then retract a support and show the descendant becomes stale |
| Retrieval and explanations | `src/v2/`, `frontend/src/` | Lexical/vector/temporal retrieval, graph expansion, authoritative validation, deterministic RRF and bounded evidence context | Compare graph and observation ablations, inspect ranks, paths, timestamps and evidence |
| Harness and evaluation | `integrations/pi`, `benchmark` | Automatic recall, observed tool outcomes, local spool, isolated benchmark banks, blinded judging, paired confidence intervals | Resume Pi across sessions; inspect captured command exit status and trace; reproduce the experiment |

These are subsystem ownership suggestions. Team members should explain and verify their own implementation contributions rather than present suggested assignments as historical work.

## Storage semantics

A single-valued slot has at most one active value. A sufficiently confident explicit correction supersedes its previous value and closes its validity interval. An unexplained competing value becomes contested. Multi-valued facts remain additive; event facts include event time in reinforcement identity. Assertions are never silently replaced by a new source. A retroactive correction can become valid before the previous belief’s inferred start; the superseded belief closes at the later of its own start and the correction date, producing an empty interval when the old belief was erroneous throughout. Recorded timestamps preserve when both beliefs entered storage.

Quotes must be exact substrings of the cited input events. Entity aliases are retained only when they occur in supporting evidence. Observation support must refer to active assertions within the same namespace. Removing or superseding a support recursively marks dependent observations stale; stale assertions are excluded from current retrieval. Consolidation creates new observations pointing to existing supports, avoiding dependency cycles.

An idempotency key identifies an immutable episode request. Reusing the key with different content fails. Durable jobs use leases and token fencing. Namespace leases allow four banks to process concurrently while preserving episode order within each bank, including after failures. Consolidation waits for queued extraction to finish. Model responses are cached separately from stored claims. Graph writes carry monotonically increasing revisions so an older projection cannot overwrite a newer assertion. Graph candidates are rechecked against PostgreSQL before use.

## V2 API

All namespaces must be explicit. Requests and responses can be inspected in the source types in `src/knowledge.rs` and `src/v2/recall.rs`. Only the evidence API is maintained; trace endpoints are under `/api/v2/traces`.

| Method and path | Purpose |
|---|---|
| `POST /api/v2/retain` | Store an episode and queue extraction; supply namespace, session_id, external_id, events and metadata |
| `POST /api/v2/recall` | Retrieve attributed evidence with query, namespace, graph/observations switches, optional as_of/from/to, token budget and optional `max_distance` relevance cutoff (see below) |
| `POST /api/v2/reflect` | Produce a bounded response citing retrieved assertion IDs |
| `GET /api/v2/status?namespace=...` | Observe queued, running and failed work plus projection progress |
| `GET /api/v2/jobs?namespace=...` | Inspect processing jobs |
| `GET /api/v2/jobs/{id}?namespace=...` | Inspect one job and its result/error |
| `POST /api/v2/jobs/{id}/retry?namespace=...` | Retry a failed job |
| `GET /api/v2/graph?namespace=...` | Inspect graph entities, assertions and edges |
| `GET /api/v2/graph/projection?namespace=...&limit=...` | Read the namespace from Neo4j: fact and entity nodes, MENTIONS and SUPPORTS relationships, counts and a `truncated` flag. `limit` is 1 to 2000 (default 400) and a value outside the range is rejected with 400, never clamped. A Neo4j failure is a 503, never an empty graph |
| `GET /api/v2/graph/memories?namespace=...&limit=...` | Read the source text from PostgreSQL: chunks that facts cite plus the newest user and assistant chunks, each with its supporting quotes and its extraction status (`succeeded`, `pending`, `running`, `failed`, `blocked` or `none`), plus namespace-wide counts of assertions by status and open extract and project jobs. `limit` is 1 to 1000 (default 200) and a value outside the range is rejected with 400. A quarter of the limit is kept for chunks no fact cites yet, so a memory that was just retained is listed even in a large namespace |
| `GET /api/v2/assertions/{id}?namespace=...` | Inspect assertion provenance |
| `POST /api/v2/assertions/{id}/retract?namespace=...` | Retract a fact and invalidate supported observations |
| `POST /api/v2/graph/rebuild?namespace=...` | Queue projection reconstruction |
| `POST /api/v2/graph/clear` | Delete everything one namespace knows. Body `{"namespace":"...","confirm":"..."}`, and `confirm` must repeat the namespace exactly or the request is a 400 |

### Recall latency

A recall costs about one embedding of the query plus a few database round trips: 65 to 75 ms for a new question on a CPU Ollama, 12 to 15 ms for a question asked before. What used to make it slow was the date planner. Any query with a word such as "when", "before", "after", "last " or a month name (matched as a substring, so "may " too) made `recall_engine` call the model for a time window, which took two to four seconds and, for ordinary questions, returned no window at all. Now `src/temporal.rs` reads a short list of time expressions itself (yesterday, today, last or this week, month or year, "N days ago", "in March 2025", "since 2025", ISO dates, "as of" a date) and an unrecognised query simply gets no window. `TEMPORAL_PLANNER` chooses the behaviour: `rules` (default, no model call), `model` (the rules first, then the old model call for queries that look temporal) or `off`. An unknown value stops the service at start. Three smaller changes: the entity lookup and the outstanding-jobs count run while the embedder works instead of after it; `OllamaEmbedder` keeps the last 256 embeddings of short texts, so asking again skips the embedder (readiness bypasses this cache, so `/healthz` still reflects Ollama); and Ollama keeps the embedding model loaded for `OLLAMA_KEEP_ALIVE` (default `1h`, it was 10 minutes), with a warm-up request at start so the first recall does not load it.

### Clearing a namespace

`POST /api/v2/graph/clear` empties one namespace in a single PostgreSQL transaction (`Store::clear_namespace` in `src/knowledge_store/maintenance.rs`): its facts, entities and aliases, edges, episodes, the events and chunks under them, any V1 memory rows left by older releases and the namespace's jobs. Traces stay, unlinked from the sessions and events they pointed at, because they are operational logs. Other namespaces are not touched. The same transaction queues a `clear_graph` job with a revision taken from the sequence the facts use, so a project job for an older fact cannot bring one back, and the endpoint also runs that clear on Neo4j before it answers, so the graph is empty when it returns. The queued job is the retry if Neo4j was unreachable, and the response says so in `graph`. A namespace with a running job (an unexpired lease) is refused with a 409, because that worker would write into rows that are gone. The graph page has a Clear button that asks for the namespace to be typed, and `scripts/clear-graph.sh NAMESPACE` does the same from a shell. The service has no authentication, so a deployment that exposes the API should not expose this route.

Readiness includes failures: pending, running, or failed jobs make a namespace unready. A submission acknowledgment means evidence was retained, not that extraction has finished.

### Recall relevance cutoff

The vector channels of recall return their nearest rows however far away they are. In a large namespace a prompt that has nothing to do with what is stored still receives the least unrelated text, and an agent that recalls before every prompt fills its context with it. The optional request field `max_distance` is a floor: a cosine distance, greater than 0 and at most 2 (0 is identical, 1 unrelated, 2 opposite). It is accepted by `/api/v2/recall` and `/api/v2/reflect`.

**Default: none.** Without the field recall behaves, and runs the same SQL candidate queries, as before, so the benchmark runner, which does not send it, is unaffected. An end to end comparison of the build before the field existed with the current one, over 80 prompts and five request shapes each (480 replies, graph on and off, `raw_only`, a larger `top_k`), found identical results, rankings and context, and `"max_distance":null` behaves as an absent field. With the field, candidates farther than `max_distance` are dropped before fusion and before each channel's row limit:

- the fact vector channel;
- the vector channel for `raw_only` requests;
- the source text passes of recall, which `include_raw` turns on for every request and which also run for evidence that extraction has not finished or when nothing else matched: the nearest vectors pass, and the any-word pass, which drops a chunk only when it has an embedding farther than `max_distance`. A chunk that has no embedding yet is kept, so text retained a moment ago still appears before the embedder has run;
- graph expansion: a fact that only the graph reached is dropped when its own embedding is farther than `max_distance`, and one with no embedding yet is kept. Without this the graph fills the context with whatever its seeds touch, however far that is from the question. The trace step `graph_expansion` lists what was dropped as `dropped_by_max_distance`.

**The field removes candidates and never reorders the rest.** A candidate that stays keeps the rank it has in each list without the field, so a list can have gaps where far candidates were, and its fusion score is unchanged. Numbering the survivors from 1 instead let the source text that remained jump over facts: with the far chunks gone from the any-word list a surviving chunk moved up several places, outscored facts that had both a word rank and a vector rank, and spent the token budget in their place. Together with the graph bound, this is what changed on 80 prompts: before, 11 relevant results that the unbounded recall packed were left out although their own distance was inside the cutoff, and now none is. Every relevant result that is left out is beyond `max_distance`.

Not gated: the word channel for facts, so an identifier such as `v2.rs` or `atlas-staging` is found even when its fact's vector is far from the question; the exact word channel for `raw_only` requests; and the temporal channel. When nothing is close enough the reply is a normal 200 with empty `results` and `context`, and reflect answers insufficient evidence.

Errors. A number that is NaN, zero, negative or above 2 is a 400 with a message. A value of the wrong JSON type, such as a string, is rejected by the request parser with a 422 that names the field, like any other mistyped field. If the query cannot be embedded no distance exists. Recall then leaves out what cannot be measured instead of returning it unfiltered: facts that match words stay, and so do source text without an embedding, while source text and graph facts that have an embedding are dropped. `degraded_reasons` says so.

The reply repeats the cutoff it used as `max_distance`, and omits the field when the request had none. A service built before the field accepts it and ignores it without a word, so restart the service after updating the Pi extension. The extension compares the echo with what it asked for and shows a warning when they differ.

The Pi extension sends `max_distance` 0.45 on the automatic recall before each prompt, the named constant `AUTOMATIC_RECALL_MAX_DISTANCE` in `integrations/pi/client.ts`, and nothing on the explicit `memory_recall` tool, because the model asked a deliberate question and should get the nearest memory.

**Calibration.** 0.45 comes from cosine distances measured with `qwen3-embedding:0.6b` on one namespace (`user:dev`, about 320 facts), not from a benchmark:

- Facts that answer a prompt were at 0.236, 0.300, 0.333, 0.386, 0.388 and 0.439.
- The best unrelated fact for a full sentence was at 0.445 to 0.564: 0.445 to 0.456 for paraphrased questions, 0.509 for small talk, 0.539 for a statement with nothing relevant stored.
- A coding task that names `src/worker.rs` had relevant facts at 0.368 to 0.462.

A wider sample of 80 prompts (55 with an answer in the namespace) shows where this stops working. The best relevant fact reached 0.465 ("where does the pre-production data live?"), and 12 of the 25 prompts without an answer had a fact or source text inside 0.45. Those are short follow-ups. "ok" is 0.415 from the nearest fact and 0.340 from the nearest source text, "continue" 0.404 and 0.385, while a one word prompt that does have an answer is as near ("staging" 0.307, "Meera" 0.448). Distance cannot tell them apart, so the cutoff mainly removes off-topic prompts and leaves some memory on very short ones: 13 of 17 follow-ups and small talk prompts still returned something at 0.45, 1250 characters on average against 6925 without a cutoff.

How the cutoff moves that trade, on the same 80 prompts:

| max_distance | answerable prompts with the target in the context (of 55) | prompts without an answer that still returned over 1000 characters (of 25) | follow-ups and small talk: mean characters returned (of 17) |
|---|---|---|---|
| none | 39 | 25 | 6925 |
| 0.40 | 50 | 1 | 353 |
| 0.42 | 50 | 1 | 436 |
| 0.45 | 49 | 6 | 1250 |
| 0.47 | 46 | 8 | 2134 |
| 0.50 | 42 | 13 | 3763 |

Each row is one cutoff, not one prompt. At result level a lower cutoff gives up some relevant results as well: of the 126 results that matched their prompt's relevance pattern, 0.42 keeps 56% and 0.45 keeps 71%, so a cutoff of about 0.42 is the stricter choice when noise on short prompts matters more than a relevant fact that sits just above it. The numbers belong to one embedding model and one namespace. Another model, or a very different namespace, has different distances and needs its own calibration: measure the nearest distance for prompts that should and should not recall something, and put the cutoff between them.

### Graph view

`GET /graph?namespace=...` serves a single self-contained page that draws what the namespace holds as a force-directed graph: memories (what was said), facts (assertions), and entities, linked by evidence, mentions and fact-to-fact relations. It polls the two graph endpoints above. Optional query parameters are `poll` (milliseconds between refreshes, default 2000, minimum 500, `0` turns live updates off), `limit` (projection nodes) and `memory_limit` (memories). It stops polling while the tab is hidden. Every failure is shown in a red banner with the server message, a truncated result is flagged, and the status strip warns when Neo4j is behind PostgreSQL or when an extraction job has failed (a failed extract job also blocks the queued jobs behind it, because the worker never skips one). The page and both endpoints are unauthenticated and show raw conversation text, so keep `BIND_ADDR` on loopback unless the network is trusted.

## Evaluation

The runner pins the cleaned LongMemEval-S dataset and upstream judgment rubric with checksums. It ingests complete histories through an allowlist that excludes questions, answers and evidence labels. Banks are isolated per question. Readers run in fresh temporary directories with tools, extensions, skills and memory disabled.

```sh
python3 benchmark/run.py fetch
# Development run, explicitly excluded from full completion:
python3 benchmark/run.py all --run development-10 --limit 10
# Freeze implementation first, then run all 500:
python3 benchmark/run.py all --run review-full
```

Stages are resumable: `prepare`, `ingest`, `retrieve`, `answer`, `judge`, `report`, and `audit`. A manifest rejects reuse after implementation/configuration changes. Raw hybrid, enhanced and full history are primary conditions; graph and consolidation ablations are included. The fixed 100-question sample receives two additional repetitions. Reports expose graded counts and paired bootstrap intervals. Human audit packets hide the condition; two reviewers fill Boolean labels, and disagreements require adjudication. Recreating a packet preserves existing reviews and rejects changed answers.

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
