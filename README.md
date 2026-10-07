# Evidence Graph Memory Engine

A Rust memory service for programming assistants and long conversations. It retains immutable evidence, extracts typed temporal assertions, derives supported observations, and retrieves attributed context through lexical, vector, temporal and graph search.

**Benchmark execution has been handed off to the project team. No accuracy improvement is claimed yet.** The original fork was merged before this enhancement. The implementation, live pilot artifacts and pending completion requirements are distinct from a completed benchmark. See the [complete benchmark runbook](benchmark/README.md).

## Run it locally

You need Docker with Compose, Python 3, [ripgrep](https://github.com/BurntSushi/ripgrep), curl, Node 22 or newer with npm, and [Pi](https://github.com/earendil-works/pi) logged in to a model provider (`pi`, then `/login`). The extraction worker runs through Pi. Rust is optional: when the host `cargo` is the pinned toolchain (`rust-toolchain.toml`) it builds with that, otherwise in a pinned container.

### 1. Start the service

```sh
./scripts/run-memory.sh
```

The script checks that the worker model works (it stops with the reason if the provider is not logged in, the model is unknown or a small test request fails, so a misconfigured worker never starts silently), starts PostgreSQL and Neo4j in Docker, creates the `memory_app` database, makes sure Ollama answers on port 11434 (it starts the compose `ollama` service if nothing does) and pulls `qwen3-embedding:0.6b` (about 640 MB the first time), builds the release binary and runs it in the foreground. Ctrl-C stops the service; the data stays in Docker volumes.

| Address | What it is |
|---|---|
| http://127.0.0.1:8080 | Observatory: inspect jobs, facts and traces |
| http://127.0.0.1:8080/swagger-ui/ | API documentation |
| http://127.0.0.1:8080/graph?namespace=user:&lt;name&gt; | Live graph of one namespace: facts, entities and the source text behind them |
| 127.0.0.1:55432, 127.0.0.1:7474 and 7687 | PostgreSQL (authoritative) and Neo4j (a rebuildable projection) |

### 2. Connect Pi to it

```sh
pi -e "$PWD/integrations/pi/extension.ts"     # this session only
pi install "$PWD/integrations/pi"             # every session, in any directory
```

Tell Pi a fact, start a new session in another directory and ask about it: the memory is per user (`user:<login name>`), not per directory. [Use with Pi](#use-with-pi) has the details, and the graph page fills in as the worker extracts the fact.

### 3. Clear a memory

```sh
scripts/clear-graph.sh user:<name>        # asks you to type the namespace; --yes skips the question
```

The Clear button on the graph page does the same. It deletes the namespace's facts, entities, source text and jobs and empties its graph. Other namespaces are not touched.

### Pi in a browser, next to the graph

`deploy/azure/run-local.sh` runs the same stack that is hosted on Azure: a page with a real Pi terminal on the left and the live graph on the right, behind one access code, with its own databases. Open http://127.0.0.1:18088/ after it prints the access code. It needs Docker Compose 2.24 or newer and a logged-in Pi, and the first run compiles the service in a container. [deploy/azure/README.md](deploy/azure/README.md) covers it, and putting it on an Azure VM.

### Configuration

| Variable | Default | Meaning |
|---|---|---|
| `BIND_ADDR` | `127.0.0.1:8080` | Listen address. The API has no authentication and the graph page shows raw conversation text, so use a non-loopback address only on a network you trust |
| `DATABASE_URL` | `postgres://memory:memory@127.0.0.1:55432/memory` | `run-memory.sh` points it at `memory_app`; tests use `memory_test` |
| `NEO4J_URI`, `NEO4J_USER`, `NEO4J_PASSWORD` | `http://127.0.0.1:7474`, `neo4j`, `password` | Compose credentials, for local development only |
| `OLLAMA_URL`, `EMBEDDING_MODEL` | `http://127.0.0.1:11434`, `qwen3-embedding:0.6b` | Query and fact embeddings (1024 dimensions) |
| `OLLAMA_KEEP_ALIVE` | `1h` | How long Ollama keeps the embedding model loaded; the service also loads it at start |
| `TEMPORAL_PLANNER` | `rules` | How a recall finds a time window: `rules` (no model call), `model` (rules first, then a model call that costs seconds) or `off` |
| `PI_PROVIDER`, `PI_MODEL` | Pi's own default | The worker model; set both to choose another. The benchmark runbook pins its own |
| `MEMORY_WORKER_CONCURRENCY` | `4` | Namespaces extracted in parallel |
| `MEMORY_MODEL_PROVIDER` | `pi` | `ollama` runs the worker on a local model (`EXTRACTION_MODEL`, default `qwen2.5:14b-instruct-q4_K_M`) |
| `MEMORY_URL`, `MEMORY_NAMESPACE`, `MEMORY_SPOOL` | | Pi extension: where the service is, which memory to use, where unsent evidence waits |

Ollama silently cuts any prompt longer than its context window (4096 tokens unless told otherwise), so the service always sends `OLLAMA_NUM_CTX` (default 16384) and `OLLAMA_NUM_PREDICT` (default 4096) and refuses to run a prompt that cannot fit. See "Running with a local model" in `docs/evidence-memory.md` for the variables, the GPU memory they cost and the failure modes. How a recall gets its speed, and what the date planner does, is in the same file under "Recall latency".

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

Automatic recall runs before the agent starts. It asks for no date window (`temporal: false`; the server's own planner is rule based and fast, a model planner is opt-in with `TEMPORAL_PLANNER=model`), and it also searches retained source text and ranks it with the extracted assertions, so a missed or misread extraction is less likely to hide something you just said. It also sets `max_distance` 0.45 (`AUTOMATIC_RECALL_MAX_DISTANCE` in `integrations/pi/client.ts`), so vector matches farther than that cosine distance are dropped and a prompt that is off topic for what is stored injects little or nothing instead of the nearest unrelated text. Very short prompts such as "ok" or "continue" sit as near to stored memory as real questions do, so some memory still comes back for them. The explicit `memory_recall` tool sets no cutoff. Restart the service after updating the extension: a service built before the cutoff existed ignores it, and the extension then shows a warning. Evidence is queued locally before submission, and each turn retains only what is new. The extension supplies `memory_recall`, `memory_remember`, and `/memory-status`.

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

`max_distance` is an optional recall field, a cosine distance greater than 0 and at most 2 (a number outside that is a 400, a value that is not a number a 422). Default: none, and recall is then unchanged. When set, vector candidates farther than it are dropped before fusion, in the fact channel, the source text passes (`raw_only`, `include_raw` and the passes for evidence that extraction has not finished) and the graph expansion; a source text match or graph fact with no embedding yet is kept, the survivors keep the ranks they had, and exact word matches on facts are kept whatever their vector distance. When nothing is close enough the reply is an empty 200, not an error, and the reply repeats the cutoff it used. For example `"max_distance":0.45`. The 0.45 comes from `qwen3-embedding:0.6b` on one namespace of about 320 facts, where relevant facts were at 0.236 to 0.439 and the best unrelated hit for a full sentence was at 0.445 to 0.564. The margin is thin at the top and does not hold for one or two word prompts, and another embedding model needs its own calibration. See [the recall relevance cutoff](docs/evidence-memory.md#recall-relevance-cutoff).

Retention acknowledges durable evidence; extraction is asynchronous. Inspect jobs and wait for readiness before an evaluation. Status exposes failures, embedding gaps and processing durations. Swagger documents both API generations.

## Verify and evaluate

```sh
./scripts/test.sh
# Live check through real Pi sessions (bills your provider; needs the service running):
python3 scripts/pi-rpc-e2e.py
# Recall latency of the running service (40 questions, then 10 requests a second):
scripts/recall-latency.py user:<name>      # restart the service first: it caches query embeddings
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
