# The exact flow of one memory

This follows one fact, "we switched payments-api from npm to pnpm", from a Pi coding session to the next session that recalls it. Every step names the file that does it.

## 1. Capture in Pi (`integrations/pi/extension.ts`)

The memory extension runs inside Pi. It does three things.

- **Tool results** are recorded the moment they happen (`tool_result` hook) with the tool name, its input, the exit code and an error flag.
- **At the end of every agent turn** (`agent_settled`) it gathers everything since the last cursor: the user's prompts, the assistant's messages and the tool results, each as `{role, content, occurred_at, metadata}` with role `user`, `assistant` or `tool`. That list is one **batch**, with an idempotency key made from the Pi session and branch, so a retry never stores it twice.
- **The batch is written to a local spool on disk and synced before any HTTP call** (`spool.ts`), then sent to `POST /api/v2/retain`. If the service is down the batch waits in the spool.

Before every agent turn (`before_agent_start`) the extension also calls `POST /api/v2/recall` with the user's prompt and injects the result as a message (step 8). Pi and its API key do nothing else: they run the coding session. The memory service never calls Pi.

## 2. Retain (`src/v2/mod.rs`, `src/knowledge_store/evidence.rs`)

`retain` stores the batch as immutable evidence in one transaction: a `raw_events` row per event, a `chunks` row per event, an `episodes` row for the batch (an **episode** is the unit of extraction) and `episode_sources` linking them in order. It queues one `extract` job for the episode. The source chunks are embedded right away, so semantic recall works before extraction has finished. Nothing is interpreted yet.

## 3. The job queue (`src/worker/mod.rs`, `src/knowledge_store/jobs.rs`)

A worker claims the `extract` job under a lease (renewed every 30 seconds) and a per-namespace lease, so two episodes of one namespace never apply at once. A lost lease cancels the work. Failures are recorded on the job and retried; they are visible at `GET /api/v2/jobs`.

## 4. Extraction by the student model (`src/worker/extraction.rs`)

1. The episode's events are loaded and the **existing-facts snapshot** is taken once: up to 80 active or contested facts of the namespace, newest first, as `{id, subject, subject_id, predicate, value, cardinality, valid_from}`.
2. Each event is **shortened for the prompt** (`src/worker/window.rs`): long output keeps its start and end, tool metadata keeps only the tool, exit code and short inputs. Consecutive events are grouped into **windows** of about 2,800 estimated tokens.
3. For each window the prompt is built from `src/worker/extract_prompt.txt` with the (possibly trimmed) snapshot and the window's events, indexed from 0. The prompt says what to keep (project setup, user preferences, decisions with reasons, owners, problems with cause and fix, dated events, open goals) and what to skip (code, chatter, guesses, and always secrets), how to write subjects, predicates and cardinality, and the exact JSON shape.
4. `OllamaJsonModel` (`src/model.rs`) sends it to the student model through Ollama's `/api/generate` with the fixed system message, `format: json` and temperature 0, after refusing a prompt that cannot fit the context.
5. The reply is parsed and **validated by the engine's rules** (`validate_claim` in `src/knowledge.rs`): required fields, allowed enums, confidence between 0 and 1, `related` only pointing at snapshot ids, and every `quotes[i]` must be an exact substring of event `source_indices[i]`, checked against the full unshortened event. A reply that fails is sent back with the error, at most twice.
6. Window-local indices are shifted to episode indices and the windows' claims are merged. If the model cannot be reached or never produces a valid reply, the job fails with that error. There is no fallback model.

For our example the claim is: subject `payments-api` (project), predicate `uses_package_manager`, value `pnpm`, cardinality `single`, `correction: true`, quote "we switched from npm to pnpm" from the user's event.

## 5. Storing claims (`src/knowledge_store/assertions.rs`)

In one transaction each claim becomes an **assertion** on an **entity**. The slot is `subject::predicate`.

- Same slot and same value already active: the existing assertion is **reinforced** and gains the new evidence.
- Different value in a `single` slot: if `correction` is true and confidence is at least 0.8, the old assertion becomes `superseded` and its validity ends at the new one's start; a `supersedes` edge links them. Otherwise both become `contested` with a `contradicts` edge. History is never deleted.
- `multiple` slots accumulate; `event` claims carry `event_at`.

Each assertion links to its evidence chunks with the verbatim quote, and the statement is embedded for semantic search. When the episode is stored, a `consolidate` job is queued.

## 6. Consolidation (`src/worker/consolidation.rs`)

The same student model reads the active facts of the subjects the episode touched and may write **observations** that combine at least two distinct facts, each citing its supports. An empty list is the usual answer. Observations are stored as assertions of kind `observation` and are validated by `validate_consolidation`.

## 7. Graph projection (`src/worker/projection.rs`)

A `project` job writes entities, assertions and their edges to Neo4j for the graph page and for graph expansion at recall time. The graph is derived: `rebuild` recreates it from PostgreSQL.

## 8. Recall (`src/v2/recall.rs`)

For a query the engine runs, concurrently where it can:

- **lexical** search over assertions and retained text,
- **semantic** search with the query embedding (`qwen3-embedding:0.6b`), optionally cut off at a maximum cosine distance (the extension uses 0.45),
- **temporal** search when the query asks about a time ("last Thursday"); the date is parsed by rules (`src/temporal.rs`), not by a model,
- **graph expansion** from the entities named in the query.

The ranked lists are fused with reciprocal rank fusion (`1/(60+rank)`) and packed into a token budget. Each hit is printed as its statement, its status and validity, and up to three evidence quotes with session, role and time. The extension injects that text before the next agent turn, labelled as evidence and not as instructions. `POST /api/v2/reflect` can instead have the student model answer a question from that evidence with cited ids; citations that are not in the evidence are rejected.

## What the student model does and does not do

It reads prompts built by the engine and writes JSON that the engine checks. It never decides what is stored: a claim whose quotes are not in the events, or that names an unknown related fact, is rejected before anything is written. The three tasks it serves are extraction, consolidation and reflection; the date parser and all retrieval are code. How it was trained and how well it does is in [slm-distill/README.md](../slm-distill/README.md).

## Known limits

- The snapshot is taken once per episode. A value changed in a later window of the same episode is stored as a correction only if the model marks `correction`; it does not see the claims of earlier windows.
- Output that is cut off at the head and tail of a long tool result is not seen by the model, so a fact that only appears in the middle of a 50 kB log is not extracted.
- A fact that only the assistant states, with no tool result or user confirmation, is stored with lower confidence but is still stored.
