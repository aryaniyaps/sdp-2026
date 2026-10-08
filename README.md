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

## Running locally (including offline)

Once prepared, the dashboard, graph, PostgreSQL, Neo4j, embeddings, extraction,
consolidation and reflection run locally without internet. **Only Pi's remote GPT
provider needs internet** to answer coding prompts. Opening the terminal does not
make those models available offline. No training environment is needed to run the app.

### Start this workstation

This checkout already has the runtime images, compiled app and both models installed.
From the repository root:

```sh
# Host Ollama serves both the browser and native stacks on this workstation.
sudo systemctl start docker ollama
./deploy/azure/run-local.sh
./deploy/azure/run-local.sh check
```

Open **http://127.0.0.1:18088/**, username `reviewer`. Read the password locally:

```sh
cat ~/.local/state/sdp-hosted-local/access.code
```

The browser API is http://127.0.0.1:18080/; health is `/healthz`, API docs are
`/swagger-ui/`, and the graph is `/graph?namespace=YOUR_NAMESPACE`.
Create projects in the dashboard; their default namespace is `project:<project name>`.
The dashboard namespace control and Pi's `/memory-namespace` synchronize after the
current agent turn finishes.

Normal startup uses saved configuration and installed images with `--pull never
--no-build`. It does not run npm, Cargo, model downloads, or refresh Pi credentials.
It checks image availability, API health, both installed models, the page, terminal and dashboard project API.
Missing prerequisites cause a failure rather than a download attempt. To stop while
keeping memory and models:

```sh
./deploy/azure/run-local.sh down
# Start again, even without internet:
./deploy/azure/run-local.sh
```

Configuration and the access code live in `~/.local/state/sdp-hosted-local/`.
The existing override uses host Ollama on port 11434, the local subnet
`10.253.202.0/24`, worker `mem-extractor:general-v1` and embedding alias
`qwen3-embedding-cpu` (CPU embeddings avoid GPU contention). Preparation preserves
these customizations. Docker volumes hold the databases and project files. Do not
remove these volumes, prune the runtime images, or delete Ollama's models if you need
offline startup. `destroy` explicitly deletes the browser stack's volumes and state.

### Prepare a new installation while online

Install Docker Engine with Compose **2.24+**, Git, Python 3, curl and OpenSSL.
Have Pi installed and authenticated (`pi`, then `/login`), with a default provider
and model selected. The browser image includes Node, Pi, ttyd, the extension, the
compiled Rust API and built frontend; host Node/Rust are unnecessary for this path.
Allow space for the worker (about **4.3 GB**), embeddings (about **640 MB**), Docker
images and build caches. Builds need substantially more space than the model weights.

```sh
# First preparation downloads/builds everything; it needs internet.
./deploy/azure/run-local.sh prepare
# Verify BEFORE disconnecting:
./deploy/azure/run-local.sh check
```

A fresh setup uses its own persistent Ollama container. With an NVIDIA GPU and the
NVIDIA Container Toolkit installed, use `GPU=1 ./deploy/azure/run-local.sh prepare`
on the first preparation. CPU inference works but is slower. `SLM_FILE=/path/to/model.gguf`
uses an existing worker file instead of downloading it. The default download is
checksum verified and pinned to Hugging Face revision
`77851da91c15fd4e20018a1b0ace13d29e7eb635`.

`prepare` also rebuilds the browser image after source changes. On an existing
installation it preserves custom overrides, credentials, model selection and volumes,
while refreshing managed proxy routes and the Pi launcher;
it checks existing models rather than replacing them. If an existing model was
removed, reinstall it into its configured Ollama while online. For this workstation:

```sh
ollama pull qwen3-embedding:0.6b
scripts/fetch-slm.sh
# Restore the aliases used by this workstation if absent:
printf 'FROM qwen3-embedding:0.6b\nPARAMETER num_gpu 0\n' | ollama create qwen3-embedding-cpu -f -
ollama cp mem-extractor mem-extractor:general-v1
```

For the fresh container-Ollama setup, its API is published at `127.0.0.1:18434`:

```sh
OLLAMA_HOST=http://127.0.0.1:18434 ollama pull qwen3-embedding:0.6b
OLLAMA_URL=http://127.0.0.1:18434 scripts/fetch-slm.sh
```

Models must reside on this machine for offline use; a remote `EXTRACTION_OLLAMA_URL`
or GPU tunnel still needs a network. `LOCAL_PORT`, `LOCAL_ENGINE_PORT`,
`LOCAL_TERMINAL_PORT`, `LOCAL_OLLAMA_PORT`, `PI_AGENT_DIR`, `PI_PROVIDER` and `PI_MODEL`
configure a fresh preparation. For an existing stack edit its saved override instead.
`SDP_LOCAL_STATE` selects another configuration directory.

### Verified offline runtime

On 2026-10-08 an isolated stack was cold-started from the installed images on a
Docker `internal` network. An outbound HTTPS request from the engine failed as
expected. The dashboard and terminal pages were checked through the internal
bridge addresses (Docker internal networks do not expose their published ports
here). The stack used this machine's existing Ollama and passed all **16** live
checks: extraction, corrections, source quotes, consolidation, Neo4j projection,
recall and cited reflection. No GPT calls were involved. The receipt is
`slm-distill/data/research-v3/offline-startup-acceptance.json` (local, ignored).
The ordinary browser startup/check and native startup also passed. The host's
internet connection was left intact; this was an isolated runtime test.

### Native API and CLI Pi

This workstation also has a native API user service at **http://127.0.0.1:8080/**.
It uses the private ignored `.env`, a prebuilt binary and separate `memory_app` data:

```sh
systemctl --user start sdp-2026-memory
systemctl --user status sdp-2026-memory
curl -fsS http://127.0.0.1:8080/healthz
# After changing its configuration or rebuilding:
systemctl --user restart sdp-2026-memory
journalctl --user -u sdp-2026-memory -n 50
# Stop it:
systemctl --user stop sdp-2026-memory
```

For a new native installation, install Node.js 22/npm, Rust 1.96.1 (or use the
script's Rust build container), and ripgrep in addition to the prerequisites above:

```sh
./scripts/run-memory.sh prepare   # online: installs dependencies/models and builds; runs in foreground
# Later, offline (Ctrl-C stops the foreground API):
./scripts/run-memory.sh
```

Do not run the foreground API and user service on port 8080 at the same time.
The root stack uses PostgreSQL port 55432 and Neo4j ports 7474/7687. Tests use
`memory_test`; the browser stack has its own databases. Stop root database containers
with `docker compose stop postgres neo4j` after stopping the native API.

To use the browser stack's memory from host Pi (GPT turns require connectivity):

```sh
MEMORY_URL=http://127.0.0.1:18080 MEMORY_NAMESPACE=user:dev \
  pi --provider openai --model gpt-5.6-sol -e integrations/pi/extension.ts
```

Use your configured provider/model if different. If credentials expire, reconnect
and use Pi's `/login`; keep credentials outside Git. The API has no authentication;
keep direct API ports on localhost or a trusted private network.

### Check readiness and troubleshoot

```sh
./deploy/azure/run-local.sh check
curl -fsS http://127.0.0.1:18080/healthz
(cd ~/.local/state/sdp-hosted-local && docker compose -p sdp-memory-local logs --tail=80 engine term)
ollama list
```

Health must report `database: true`, `embedder: true`, `degraded_mode: false` and
the intended worker model. Installed model checks are not a semantic accuracy test.
For actual extraction, retain the API example below, then inspect
`/api/v2/jobs?namespace=example` until extraction, consolidation and projection finish;
recall should return supported facts. A successful retain alone only means queued.

If port 8080 is occupied, use the existing native service or stop it before launching
the foreground script. If the browser stack cannot reach host Ollama, check that the
Ollama service is running and its saved host address/subnet still matches. Keep the
existing NetworkManager Docker-bridge exclusions. Never fix this by deleting memory
volumes. Rebuild changes while online; normal startup deliberately runs the last
prepared application image/binary.

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

By default, local Pi sessions use `project:<repository basename>:<root path digest>` (or the working directory outside Git). Sessions within one repository share memory; unrelated project roots are isolated. Set `MEMORY_NAMESPACE` to explicitly share memory across projects, or switch the active namespace during a Pi session with `/memory-namespace [namespace]` (without an argument, Pi prompts for one). Switching namespaces marks a retention boundary so earlier conversation is not copied into the new namespace. Clearing also advances that boundary so cleared conversation is not replayed on the next turn. `MEMORY_URL` changes the service address and `MEMORY_SPOOL` changes the local queue location. The extension also provides `memory_recall`, `memory_remember`, `/memory-status`, and `/memory-clear [namespace]`. Clear prompts you to type the namespace exactly, clears server data, then discards queued local batches for that namespace.

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

## The worker model

The general-purpose memory worker (extraction, consolidation and reflection) uses a fine-tuned Qwen3-4B-Instruct-2507 served by Ollama. Its corpus includes everyday and professional conversations labeled by a larger teacher, using the engine's shared prompts and acceptance rules; see [slm-distill](slm-distill/README.md) for the pipeline and its measured results. Pi and its API key are used only for the live coding session.

`scripts/fetch-slm.sh` downloads the release GGUF and verifies `SHA256SUMS`, then installs it into any Ollama over HTTP (`OLLAMA_URL`). `SLM_REVISION` selects an explicit Hugging Face commit; `SLM_FILE` installs an already downloaded local GGUF. The service reads one episode as windows of a few thousand tokens and checks every claim's quotes against the full events. If the model cannot be reached, extraction jobs fail with a clear error and are retried; nothing falls back to another model. To lend a GPU workstation to the Azure stack, see [deploy/azure](deploy/azure/README.md#lend-a-gpu-to-the-azure-stack).

## Configuration

| Variable | Purpose |
| --- | --- |
| `DATABASE_URL` | PostgreSQL connection |
| `NEO4J_URI` | Neo4j HTTP endpoint; empty disables graph projection |
| `OLLAMA_URL` | Ollama endpoint |
| `EMBEDDING_MODEL` | Embedding model, default `qwen3-embedding:0.6b` |
| `EXTRACTION_MODEL` | Worker model, default `mem-extractor` |
| `EXTRACTION_OLLAMA_URL` | Ollama that serves the worker model, default `OLLAMA_URL` |
| `OLLAMA_NUM_CTX`, `OLLAMA_NUM_PREDICT` | Context window and output cap of worker calls; defaults 16384 and 4096 (the stacks here set 12288 and 3072) |
| `MEMORY_WORKER_CONCURRENCY` | Extraction workers, default 4 |
| `TEMPORAL_PLANNER` | `rules` (default), `model`, or `off` |
| `BIND_ADDR` | API address, default `127.0.0.1:8080` |

Browser preparation resolves Pi's configured default model for the terminal only. The memory worker always uses Ollama. Direct binary startup uses the defaults in `src/main.rs`.

## Checks

```sh
./scripts/test.sh
python3 scripts/test_local_stack.py
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

## Workstation training and historical verification

The following records describe the 2026-10-08 setup and evaluation; rerun the checks
for current inference results. Training is optional and separate from offline startup.

### Installed development and training tools

Rust 1.96.1 with rustfmt/clippy, Node dependencies for frontend and Pi integration, Playwright Chromium, Ollama, pgvector PostgreSQL and Neo4j containers, Pi 1.0.2, benchmark datasets, Python 3.12 training virtualenv, CUDA PyTorch, transformers/PEFT, and the checksum-verified pinned llama.cpp converter.

```sh
source slm-distill/.venv/bin/activate
python -c 'import torch; print(torch.__version__, torch.cuda.get_device_name(0))'
python -m pytest slm-distill -q
```

The host ROS `PYTHONPATH` must be cleared for this environment; its activation script does that. With direct virtualenv commands use `env -u PYTHONPATH`.

### General-purpose replacement worker

The configured `slm-v1` release was unavailable. The first replacement used Qwen3-1.7B with deterministic examples followed by teacher-labelled conversations. Its strong synthetic results did not establish general-purpose accuracy. Subsequent experiments use Qwen3-4B-Instruct-2507, broader everyday-memory conversations, source-grounded extraction, label audits and separate held-out evaluations. Coding conversations are one application of the memory engine, not its scope.

Both running services now use `mem-extractor:general-v1` (Qwen3-4B-Instruct-2507 with the trained adapter, Q8_0), context 12,288 and output cap 3,072. The default `mem-extractor` alias points to the same verified GGUF (`5f3d769b350655b4dc316953cbd362c92fa573ae3aabec5c77725f19cf6f0e36`). The older `memex-extractor:local-v1-cpu` alias remains available for rollback. This is an experimental multitask release: held-out extraction regressed against the base model, while consolidation and reflection improved. The executed [iteration notebook](slm-distill/Finetuning_Iteration_Report.ipynb) records measured results and limitations.

Adapters, provenance and trainer states: `slm-distill/out/` (ignored).
Datasets, teacher cache and evaluation receipts: `slm-distill/data/` (ignored).

The following commands reproduce the earlier 1.7B experiments, not the final 4B release:

```sh
python slm-distill/bootstrap_dataset.py
PI_TEACHER_COMPACT=1 python slm-distill/run_pi_corpus.py
python slm-distill/train.py --sets slm-distill/data/bootstrap-sets --out slm-distill/out/local-bootstrap --epochs 2
python slm-distill/refine_local.py
python slm-distill/export.py --adapter slm-distill/out/local-refined/final --name memex-extractor --quant q8_0
python slm-distill/evaluate_bootstrap.py --model memex-extractor
python slm-distill/evaluate_local.py --model memex-extractor:local-v1 --label local-v1
```

Training and inference share an 8 GB GPU. Both backends currently use `qwen3-embedding-cpu`, an alias with `num_gpu 0`, so embedding calls do not interfere with training. Worker GPU inference must also be unloaded before training. Worker generation explicitly sets `think: false` to match the no-thinking training format.

The original fine-tuned model has not been recovered. This replacement must be assessed from its own evaluation receipts. Synthetic schema acceptance is not a claim of real conversation accuracy.

### Verify the deployed memory engine

The live check creates its own namespace and preserves its receipts. It exercises everyday preferences and an explicit sister relationship, a home-city correction, supported derived observations, Neo4j projection, recall and cited reflection. It requires actual source quotes, checks that the old city is not still active, and rejects the unsupported addition of a country absent from the fixture. It fails when a required check fails; merely accepting a retain request is insufficient.

```sh
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/verify_live_memory.py --url http://127.0.0.1:18080
```

The default receipt is `slm-distill/data/research-v3/final-acceptance.json`. Check its `deployment_identity_before.health.worker_model`, `passed`, per-check outcomes and timestamps before interpreting it as evidence for a particular release. PostgreSQL remains authoritative; Neo4j is the derived graph projection. Extraction and consolidation are asynchronous, so inspect `/api/v2/jobs?namespace=YOUR_NAMESPACE` until they finish.

The final native acceptance passed all 16 checks. Its first reflection attempt exposed an invalid citation UUID; the deployed decoder now constrains citations to supplied evidence IDs. The second inference run passed after correcting a harness attribution false negative (resolved subject Maya Sen); `final-acceptance.json` explicitly records that same-output re-audit rather than claiming fresh inference. Synthetic screenshots are `slm-distill/data/research-v3/final-graph.png` and `final-graph-observations.png`.

The original browser `user:dev` queue also completed: 43 extraction, 43 consolidation and 160 projection jobs succeeded, with no pending or failed jobs. Eight active derived observations each have at least two distinct supporting facts. The Neo4j projection contains 112 nodes and 197 relationships, including 21 support/derivation edges. All 142 source-quote links match their stored source chunks verbatim. These are a point-in-time recovery receipt (`original-queue-final.json`), not a general semantic-accuracy benchmark.


## References

The project draws on [Hindsight](https://github.com/vectorize-io/hindsight), [Graphiti](https://github.com/getzep/graphiti), [Supermemory](https://supermemory.ai/docs), [Pi](https://github.com/earendil-works/pi), and [LongMemEval](https://github.com/xiaowu0162/LongMemEval). These are references for the design and evaluation, not claims that this implementation matches those projects.
