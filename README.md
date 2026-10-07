# Memory Engine

Memory Engine stores conversation history for a coding assistant and retrieves relevant parts in later sessions. It is our SDP project, built with Rust, PostgreSQL, Neo4j, and a React frontend.

The main idea is to keep the original conversation as evidence instead of storing only a generated summary. Facts link back to the text they came from. When a user corrects something, the previous fact stays in the history with its validity dates.

## What it does

- Saves user, assistant, and tool messages in separate namespaces.
- Extracts facts in the background and keeps their source quotes.
- Tracks corrections, conflicting facts, and dated events.
- Combines text search, vector search, and graph connections during recall.
- Returns cited context within a token budget.
- Connects to Pi to recall earlier sessions and save new interactions.

The dashboard lets you save an interaction, ask a question, inspect a fact's evidence, and view processing traces. The graph page shows memories, facts, and entities, with search, zoom, and node details. Both pages use React; forms, cards, tables, and dialogs use React Bootstrap. The graph layout and canvas renderer live separately from the page components.

## Running locally

For the browser demo with a Pi coding session beside the live graph, run:

```sh
./deploy/azure/run-local.sh
```

Open http://127.0.0.1:18088/ with user `reviewer`; the access code is in `~/.local/state/sdp-hosted-local/access.code`. This builds the React demo, starts the browser terminal and memory services, and uses Pi's configured default provider and model. The namespace control targets both the Pi session and graph; switching directories starts another session with the same memory. This stack keeps its own databases. See [the demo setup guide](deploy/azure/README.md) for configuration and stop commands.

You need Docker with Compose, Node.js 22 with npm, Python 3, and Pi with an authenticated model provider. The startup script uses Rust 1.96.1 if it is installed locally; otherwise it builds the backend in a Rust container.

```sh
./scripts/run-memory.sh
```

The script builds the frontend, checks that the Pi worker can make a request, starts PostgreSQL and Neo4j, and prepares the Ollama embedding model. The first run can take a while because it downloads images and model weights.

Open these URLs after the service starts:

| Page | URL |
| --- | --- |
| Dashboard | http://127.0.0.1:8080 |
| Graph | http://127.0.0.1:8080/graph?namespace=review |
| API documentation | http://127.0.0.1:8080/swagger-ui/ |
| Health | http://127.0.0.1:8080/healthz |

The script uses `memory_app` for application data. The test script uses `memory_test`. PostgreSQL listens on port 55432 and Neo4j on 7474. Development credentials are in `docker-compose.yml`.

To run extraction with a local Ollama model instead of Pi, use `./scripts/review.sh`. It requires a running Ollama installation and downloads the extraction and embedding models before starting the API through Compose. Model sizes and context limits are explained in [the backend notes](docs/evidence-memory.md#running-with-a-local-model).

The API has no authentication. Keep it on localhost or a trusted private network; the UI exposes stored conversation text.

## Frontend development

Start the backend first, then run:

```sh
cd frontend
npm ci
npm run dev
```

Vite serves the UI at http://127.0.0.1:5173 and proxies API requests to the backend on port 8080. Changes to React components update without rebuilding Rust.

For a backend build outside the startup scripts, build the frontend first:

```sh
(cd frontend && npm ci && npm run build)
cargo build --locked
```

Rust embeds the generated HTML, JavaScript, and CSS in the binary. Rebuild the frontend and backend to include UI changes in that binary. Both Dockerfiles do this automatically.

## Project layout

```text
src/
  api.rs                 health, traces, metrics, and API docs
  web.rs                 serves the built React app
  v2/
    mod.rs               evidence API routes and job status
    graph.rs             graph inspection and maintenance
    recall.rs            retrieval, ranking, and context packing
    reflect.rs           answers grounded in retrieved evidence
  knowledge_store/
    evidence.rs          stores conversation batches
    assertions.rs        facts, corrections, and observations
    jobs.rs              queue leases, retries, and completion
    maintenance.rs       namespace cleanup
  worker/                job processing and extraction prompts
  graph.rs               Neo4j client
  model.rs               Pi and Ollama inference
  providers.rs           embeddings and query cache
  temporal.rs            date and time query rules
frontend/src/
  components/            dashboard panels
  hooks/                 request state and cancellation
  graph/                 React graph page, canvas viewer, and layout
  api.ts                 typed API client
migrations/              PostgreSQL schema changes
integrations/pi/         Pi extension and its tests
benchmark/               LongMemEval runner and protocol tests
slm-distill/             separate local-model training experiment
scripts/                 startup, verification, and demo commands
```

PostgreSQL is the source of truth. Neo4j holds a projection for graph traversal. Workers pick up durable jobs, and a failed graph write can be retried without losing the original evidence. A namespace keeps one user's memory separate from another's.

The V1 memory API, deterministic demo controls, and original resolver have been removed. Traces now use `/api/v2/traces`. Existing migrations are kept unchanged so older databases can upgrade; namespace cleanup also removes rows left by the old resolver. The benchmark no longer runs its retired `legacy` condition.

## Using Pi

Load the extension for one session:

```sh
pi -e /absolute/path/to/sdp-2026/integrations/pi/extension.ts
```

Or install it for all sessions:

```sh
pi install /absolute/path/to/sdp-2026/integrations/pi
```

The extension recalls relevant memory before a turn and saves new messages and tool output afterwards. It writes evidence to a local spool before submitting it, so a temporary server failure does not lose the batch. Injected recall context is excluded from retention.

By default, sessions share `user:<login name>`. Set `MEMORY_NAMESPACE` for a separate project or experiment, or switch the active namespace during a Pi session with `/memory-namespace [namespace]` (without an argument, Pi prompts for one). Switching namespaces also keeps retention cursors separate, so the current conversation can be retained in each namespace. `MEMORY_URL` changes the service address and `MEMORY_SPOOL` changes the local queue location. The extension also provides `memory_recall`, `memory_remember`, `/memory-status`, and `/memory-clear [namespace]`. Clear prompts you to type the namespace exactly, clears server data, then discards queued local batches for that namespace.

Installing it sends session content, including tool output, to the configured service. Use `pi remove` to uninstall it.

## API example

Save some evidence:

```sh
curl http://127.0.0.1:8080/api/v2/retain \
  -H 'content-type: application/json' \
  -d '{"namespace":"example","session_id":"one","external_id":"event-1","events":[{"role":"user","content":"Ada prefers Python for programming.","occurred_at":"2026-01-01T12:00:00Z"}]}'
```

Check processing and recall it:

```sh
curl 'http://127.0.0.1:8080/api/v2/status?namespace=example'

curl http://127.0.0.1:8080/api/v2/recall \
  -H 'content-type: application/json' \
  -d '{"namespace":"example","query":"Which language does Ada prefer?","max_tokens":2048}'
```

Retention returns once evidence is saved. Extraction happens asynchronously, so a successful retain response does not mean every fact has been extracted yet. Status and traces show pending and failed work.

Recall accepts optional time bounds, graph and observation switches, and `max_distance` for vector relevance filtering. Automatic Pi recall uses a distance cutoff of 0.45, calibrated on a small sample with the default embedding model. It is imperfect for short prompts and needs recalibration for other models. See [the recall notes](docs/evidence-memory.md#recall-relevance-cutoff) for details.

To clear a namespace:

```sh
./scripts/clear-graph.sh example
```

This deletes its evidence and facts and queues a graph cleanup. The command asks you to confirm the namespace.

## Configuration

| Variable | Purpose |
| --- | --- |
| `DATABASE_URL` | PostgreSQL connection |
| `NEO4J_URI` | Neo4j HTTP endpoint; empty disables graph projection |
| `OLLAMA_URL` | Ollama endpoint |
| `EMBEDDING_MODEL` | Embedding model, default `qwen3-embedding:0.6b` |
| `PI_PROVIDER`, `PI_MODEL` | Model used by the Pi extraction worker |
| `MEMORY_MODEL_PROVIDER` | Set to `ollama` to use local inference |
| `EXTRACTION_MODEL` | Local model, default `qwen2.5:14b-instruct-q4_K_M` |
| `OLLAMA_NUM_CTX`, `OLLAMA_NUM_PREDICT` | Local context window and output cap; defaults 16384 and 4096 |
| `MEMORY_WORKER_CONCURRENCY` | Extraction workers, default 4 |
| `TEMPORAL_PLANNER` | `rules` (default), `model`, or `off` |
| `BIND_ADDR` | API address, default `127.0.0.1:8080` |

The startup script resolves Pi's configured default model when provider and model are not specified. Direct binary startup uses the defaults in `src/main.rs`.

## Checks

```sh
./scripts/test.sh
```

This builds and tests the frontend, starts the test databases, runs Rust formatting, Clippy and tests, checks the benchmark protocol, and tests the Pi extension.

For frontend-only work:

```sh
cd frontend
npm run check
npm test
npm run test:e2e
npm run format:check
```

The browser tests use mocked API responses so they can run without model inference. Install Chromium once with `npx playwright install chromium`.

The live Pi demo needs the service running and makes requests to your model provider:

```sh
python3 scripts/pi-rpc-e2e.py
```

## Evaluation status

The evaluation runner compares raw hybrid retrieval, the evidence memory engine, and full conversation history. It also supports graph and consolidation ablations, repeated answers, judging, and human review.

The full LongMemEval-S experiment is still pending. Pilot artifacts are not evidence of an accuracy improvement. Follow [the benchmark runbook](benchmark/README.md) for the dataset, commands, saved artifacts, and reporting requirements. Use a new run name after code or configuration changes; the runner rejects incompatible manifests.

Other notes:

- [Backend design and retrieval](docs/evidence-memory.md)
- [Subsystem review](docs/subsystem-review.md)
- [Local model distillation experiment](slm-distill/README.md)
- [Azure deployment](deploy/azure/README.md)

## References

The project draws on [Hindsight](https://github.com/vectorize-io/hindsight), [Graphiti](https://github.com/getzep/graphiti), [Supermemory](https://supermemory.ai/docs), [Pi](https://github.com/earendil-works/pi), and [LongMemEval](https://github.com/xiaowu0162/LongMemEval). These are references for the design and evaluation, not claims that this implementation matches those projects.
