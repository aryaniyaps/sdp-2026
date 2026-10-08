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

## End-to-end walkthrough

This example follows a fictional person's information through retention,
extraction, consolidation, corrections and recall. It also covers conflicting
reports and retrieval from a new Pi session. The API responses and graph expose
the evidence behind each stage.

### Setup

1. Start the browser stack and run `./deploy/azure/run-local.sh check` as described
   above. Check `/healthz` for the actual worker model and degraded state.
2. Open the Pi console at **http://127.0.0.1:18088/** and a separate terminal in
   this repository. Use the browser stack's API at **18080** throughout; the native
   service at 8080 has separate data.
3. Use a fresh namespace for each run to keep existing project memory intact.
   Model wording, extraction counts, observations and latency can vary. The checks
   below describe expected behavior; inspect the actual results at each stage.
4. Keep the responses for comparison between runs. If inference fails, inspect
   the current job error and distinguish it from any earlier successful result.

Paste this setup into the terminal. It needs only Python 3 and the running API.
`memory_request` makes one request, prints the response, and saves it under a temporary
receipt directory. Supplying JSON makes a POST; omitting it makes a GET.

```sh
export WALKTHROUGH_API=http://127.0.0.1:18080
export WALKTHROUGH_NS="walkthrough:$(date -u +%Y%m%dT%H%M%SZ)"
export WALKTHROUGH_RECEIPTS="$(mktemp -d /tmp/memory-walkthrough.XXXXXX)"
memory_request() {
  python3 - "$@" <<'PYREQUEST'
import datetime, json, os, pathlib, sys, urllib.parse, urllib.request
route = sys.argv[1]
ns = os.environ["WALKTHROUGH_NS"]
body = None
if len(sys.argv) > 2:
    payload = json.loads(sys.argv[2])
    payload["namespace"] = ns
    body = json.dumps(payload).encode()
else:
    route += ("&" if "?" in route else "?") + urllib.parse.urlencode({"namespace": ns})
request = urllib.request.Request(os.environ["WALKTHROUGH_API"] + route, data=body,
                                 headers={"Content-Type": "application/json"})
with urllib.request.urlopen(request, timeout=600) as response:
    value = json.load(response)
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
name = sys.argv[1].split("?")[0].strip("/").replace("/", "-")
path = pathlib.Path(os.environ["WALKTHROUGH_RECEIPTS"]) / (stamp + "-" + name + ".json")
path.write_text(json.dumps(value, indent=2) + "\n")
print(json.dumps(value, indent=2))
print("Saved:", path, file=sys.stderr)
PYREQUEST
}
printf 'Namespace: %s\nGraph: http://127.0.0.1:18088/graph?namespace=%s\nReceipts: %s\n' \
  "$WALKTHROUGH_NS" "$WALKTHROUGH_NS" "$WALKTHROUGH_RECEIPTS"
memory_request /healthz
```

Open the printed graph URL. Use **Fit**, **Search nodes**, and the **Memories**,
**Facts**, and **Entities** toggles to inspect one layer at a time. Click nodes
for details; hover over purple arrows to read their relation. Keep the namespace
identical in the terminal, graph and Pi.

### Concepts

| Term | Meaning |
| --- | --- |
| Evidence / memory | The original retained message, role, timestamp and source text. Click a memory node to see the facts it supports. |
| Claim | The extractor's proposed subject, predicate, value, kind, confidence and source quotes. The job's `result.decisions` records accepted actions and assertion IDs; inspect those assertions for the stored fields. The public job response does not expose the full raw model proposal. |
| Assertion / fact | A stored, validated claim with an ID, status and validity dates. Click a fact and inspect its assertion API response. “Validated” means structural/source checks, not proof that the speaker is truthful. |
| Entity | A person, place, technology or other subject mentioned by facts. Entity links connect related knowledge. |
| Observation | A derived assertion produced by consolidation, supported by other assertions. Follow its support relations back to source-backed facts. It is an inference, not a direct quote. |
| Correction | A new accepted value supersedes a previous belief while retaining its history. |
| Contradiction | Incompatible values remain contested when there is no sufficiently confident explicit correction. Both claims retain their evidence. |
| Recall / reflection | Recall returns ranked evidence and packed context; reflection generates an answer with citations to retrieved IDs. Pi uses recall to inform its own response. |

### 1. Retain evidence

Use the API for the first part so the source is exactly what you type, without
an assistant response adding extra facts. This is the same retention API used by Pi.

```sh
memory_request /api/v2/retain '{"session_id":"maya-story","external_id":"initial","events":[{"role":"user","content":"My name is Maya Sen. I live in Pune. I prefer vegetarian meals. I avoid meat when choosing restaurants. My sister is Leela Sen.","occurred_at":"2026-10-08T09:00:00Z"}]}'
memory_request /api/v2/jobs
memory_request /api/v2/status
```

The response includes `episode_id`, `job_id` and `created`: evidence is durable, but extraction
may still be running. Repeat the jobs/status requests until work finishes.
Inspect `extract`, `consolidate` and `project` jobs and their `status`, `attempts`,
`result` and `error`. Jobs may finish too quickly to catch every intermediate state.

**Checkpoint:** jobs have succeeded, status is `ready: true`, and the graph has
source-backed assertions. An empty namespace can also report ready, so readiness
alone is insufficient. Inspect `embedding_gaps` too. Wait for this stage before
submitting the next episode; sequential episodes make the history easy to follow.

### 2. Inspect facts and source evidence

Search for **Maya** or **Pune**. Click the memory and read its original text, then
click the residence fact to inspect its source quote. The same data is available
through the API:

```sh
memory_request /api/v2/graph/memories
memory_request /api/v2/graph
# Replace the value with the residence assertion's actual UUID from the graph response.
ASSERTION_ID='paste-assertion-uuid-here'
memory_request "/api/v2/assertions/$ASSERTION_ID"
```

Inspect `subject`, `predicate`, `value`, `kind`, `status`, `valid_from`,
`valid_to`, `sources` and `relations`. Each source includes a `chunk_id`, exact
`quote`, `role`, `session_id` and `occurred_at`. Compare the quote to the memory
text to check the assertion's provenance.

The graph JSON includes assertions and edges; its inspection endpoint is limited
to 100 assertions. Projection and memory endpoints also have limits, so use this
small namespace to avoid truncation. One graph view may not contain a large
account's entire history.

### 3. Inspect derived observations

Look for an observation about Maya's food preferences. In the graph legend,
observations have a distinct style within the **Facts** layer. Select one and
follow its support relations to the vegetarian preference and avoiding-meat facts.
In `/api/v2/graph`, locate `kind: "observation"` and its `supports`/`derives` edges;
inspect the supporting assertion IDs with the assertion endpoint.

**Checkpoint:** the observation is active and has at least two distinct supporting
facts, each traceable to evidence. Check that the inference follows from its
supports: a vegetarian preference, for example, does not establish an allergy.
If consolidation produces no observation, inspect its job result. A successful
job need not produce new knowledge.

### 4. Correct a fact and query its history

```sh
memory_request /api/v2/retain '{"session_id":"maya-story","external_id":"move","events":[{"role":"user","content":"I am Maya Sen. Correction: I moved from Pune to Chennai today. My current home city is Chennai, not Pune. My vegetarian meal preference has not changed.","occurred_at":"2026-10-08T10:00:00Z"}]}'
memory_request /api/v2/jobs
memory_request /api/v2/status
# Repeat these after jobs finish; inspect both city assertions.
memory_request /api/v2/graph
memory_request /api/v2/recall '{"query":"Where does Maya Sen currently live?","max_tokens":2048,"reference_date":"2026-10-08T11:00:00Z"}'
memory_request /api/v2/recall '{"query":"Where did Maya Sen live?","as_of":"2026-10-08T09:30:00Z","max_tokens":2048}'
```

**Checkpoint:** Chennai is active, the old Pune residence is superseded with a
closed `valid_to`, and a `supersedes` relation connects the change. The current
recall should support Chennai; the historical recall should support Pune if the
extracted validity interval covers 09:30. Verify the actual dates rather than
assuming them. `as_of` queries validity time, not a replay of the database as it
was recorded at that instant.

An explicit correction to a single-valued slot requires extracted confidence of
at least **0.8** to supersede automatically. Extraction still has to recognize
the same subject/predicate and correction. If it does not, inspect the stored
assertions and job decisions for the mismatch. Dependent observations can become
stale when their support changes.

### 5. Handle conflicting reports

Use a separate person to keep this conflict independent of Maya's correction.
Submit these **one at a time**, waiting for jobs after each:

```sh
memory_request /api/v2/retain '{"session_id":"conflict-a","external_id":"arun-a","events":[{"role":"user","content":"Arun Das currently lives in Jaipur.","occurred_at":"2026-10-08T11:00:00Z"}]}'
memory_request /api/v2/jobs
# Wait for completion before submitting the second report.
memory_request /api/v2/retain '{"session_id":"conflict-b","external_id":"arun-b","events":[{"role":"user","content":"Arun Das currently lives in Kochi.","occurred_at":"2026-10-08T11:00:00Z"}]}'
memory_request /api/v2/jobs
# After completion:
memory_request /api/v2/graph
memory_request /api/v2/recall '{"query":"Where does Arun Das live?","max_tokens":2048}'
```

**Checkpoint:** for the same single-valued residence slot, both incompatible
assertions are `contested` and linked by `contradicts`; inspect both sources.
Two conflicting reports do not establish which one is correct.
This depends on the model extracting a shared slot without marking a correction.
If it chooses different predicates or treats the second report as a correction,
inspect the stored assertions and job decisions to identify the mismatch.
`contested` facts remain eligible for recall; check whether retrieval returns both.

### 6. Retrieve evidence and generate cited answers

```sh
memory_request /api/v2/recall '{"query":"What meals does Maya Sen prefer?","max_tokens":2048,"graph":true,"observations":true}'
memory_request /api/v2/reflect '{"query":"What meals does Maya Sen prefer?","max_tokens":2048}'
memory_request /api/v2/graph/projection
memory_request '/api/v2/traces?limit=10'
```

In the recall response, inspect `results`, their `sources`, `ranks`, `paths`, the packed
`context`, `estimated_tokens`, `temporal_plan`, `degraded_reasons` and `trace_id`.
A path may be empty if a result came directly from text/vector retrieval.
In the reflection response, inspect `answer`, `citations` and `insufficient_evidence`; resolve
citation IDs against `recall.results` and inspect their source quotes.

PostgreSQL holds the authoritative evidence/assertions while Neo4j
holds the graph projection. The projection endpoint demonstrates actual Neo4j
nodes/relationships; a PostgreSQL graph response alone does not prove that
projection succeeded. Traces make retrieval and worker behavior inspectable.

For an optional comparison, repeat recall with `graph: false` and
`observations: false`, then with `raw_only: true`. These isolate capabilities;
they are not a lexical-only baseline or an accuracy benchmark, and a simple
question may return the same answer in every configuration.

### 7. Recall memory in a new Pi session

In the browser console, use `/memory-namespace` with the **exact printed
`WALKTHROUGH_NS` value** (shell variables do not expand inside Pi). Wait until the
console and graph show the same namespace, then run `/memory-status`.

Ask Pi:

> “Use memory to tell me where Maya Sen currently lives and what food she prefers.
> Cite the evidence you retrieve; do not guess missing details.”

Click **New session** in the console. Confirm the namespace again and ask:

> “What is the name of Maya Sen's sister? Retrieve the evidence from memory.”

Inspect the `memory_recall` result if Pi invokes the tool, or inspect the namespace's
recall traces for automatic pre-turn recall. The new session has not been given
the biography; a supported answer should identify Leela Sen from retained evidence.
Check `/memory-status` after the turn for retention/spool state. Pi answering
successfully without visible retrieved evidence is not enough to prove memory use.
The API and graph work locally; Pi's configured remote model requires
internet and working authentication.

### Verification and troubleshooting

For the existing automated live acceptance check, preserve a separate receipt:

```sh
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/verify_live_memory.py \
  --url "$WALKTHROUGH_API" --out "$WALKTHROUGH_RECEIPTS/live-acceptance.json"
```

This uses its own `acceptance-general:...` namespace, not `WALKTHROUGH_NS`, and needs the
training environment's Python dependencies and access to the configured Ollama
(default `http://127.0.0.1:11434`; use `--ollama-url` if different). It checks
extraction, source evidence, observations, correction, projection, recall and
reflection. It does **not** verify the ambiguous-conflict or new-Pi-session steps;
check those separately. Read `passed`, individual checks, timestamps and model
identity, then open the receipt's namespace in the graph.

| What goes wrong | What to inspect next |
| --- | --- |
| Retain succeeded but no facts appear | `/api/v2/jobs`: evidence is saved before inference. Inspect failed jobs' `error` and `result`; inspect the memory node's extraction state. |
| Fact exists in API but not graph | Confirm namespace; inspect projection jobs and `/api/v2/graph/projection`. |
| Wrong/unsupported fact | Compare the assertion's quotes with the original text and the extraction job decisions. Source matching does not guarantee semantic accuracy. |
| Recall is empty or degraded | Check namespace, worker/embedding health, `embedding_gaps`, `degraded_reasons`, temporal filters and trace. |
| Pi does not retrieve the saved information | Confirm `MEMORY_URL` points to 18080, the exact namespace, `/memory-status`, and recall traces. |
| A job failed | Fix the reported cause first, then retry that job as shown below; retain the failure receipt. |

```sh
# Replace with the failed job's UUID. The namespace is added by the helper.
FAILED_JOB_ID='paste-failed-job-uuid-here'
memory_request "/api/v2/jobs/$FAILED_JOB_ID/retry" '{}'
memory_request /api/v2/jobs
```

The complete flow is: original memory → exact quote → assertion → supported
observation → correction/history → contested conflict → cited recall →
new-session use. Record which checks passed, failed or were not run alongside
the saved responses. Keep receipts outside Git and do not include credentials.

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
