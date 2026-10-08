# Pi session and live graph demo

The demo at `/demo` is a React page with a real Pi terminal beside the memory graph. Switch between `payments-api`, `mobile-app`, and `scratch` to start fresh coding sessions that share the selected namespace. Applying another namespace restarts the terminal and opens that namespace in the graph. Pi's `/memory-namespace` and `/memory-clear` commands are available in the terminal; after changing namespaces inside Pi, select the same namespace in the page to view its graph.

The UI lives in `frontend/src/demo`: `DemoPage.tsx` owns the controls and panes, `session.ts` builds the terminal and graph URLs, and `demo.css` handles the layout. The graph retains its separate React page and canvas modules. There is no separate static landing page.

## Run locally

With Docker Compose, Python 3, openssl, curl, and an authenticated Pi installation:

```sh
./deploy/azure/run-local.sh
```

Open http://127.0.0.1:18088/ and sign in as `reviewer`. The script prints the path to the access code, normally `~/.local/state/sdp-hosted-local/access.code`. It builds the frontend, backend, and Pi image, then starts the terminal, proxy, databases, and an Ollama that holds the embedding model and the worker model (downloaded once, about 1.8 GB, from the project's GitHub release; `SLM_FILE` uses a GGUF you already have, `GPU=1` gives that Ollama your NVIDIA GPU). These databases are separate from the stack started by `scripts/run-memory.sh` and the root Compose file.

The terminal uses the provider and model in Pi's `settings.json`; set both `PI_PROVIDER` and `PI_MODEL` to override them. The memory worker never uses Pi or its login: it is the fine-tuned student model served by Ollama. `PI_AGENT_DIR` changes the source Pi login directory; `LOCAL_PORT` changes the browser port, `LOCAL_TERMINAL_PORT` changes the loopback terminal port (default 7681), `LOCAL_ENGINE_PORT` changes the loopback API port (default 18080), and `SDP_LOCAL_STATE` changes the generated state directory. Pi's coding requests use the chosen provider; Ollama supplies the local embeddings and the worker model.

```sh
./deploy/azure/run-local.sh down       # stop the demo, retaining memory
./deploy/azure/run-local.sh destroy    # delete the demo stack and its memory
```

For frontend development, start the demo stack and run `MEMORY_API_URL=http://127.0.0.1:18080 npm run dev` in `frontend`. Open http://127.0.0.1:5173/demo; Vite proxies the terminal and WebSocket to port 7681. The API proxy then uses the demo engine. Without `MEMORY_API_URL`, Vite targets the root backend on port 8080. `PI_TERMINAL_URL` overrides its terminal target.

## Services and modules

| Service or file | Responsibility |
| --- | --- |
| `term` | ttyd starts Pi with the memory extension for each browser connection. Repeated URL arguments pass the project directory and namespace to `pi-session.sh`. |
| `engine` | Stores evidence, extracts facts with the worker model, serves the React UI, and exposes graph APIs internally. It has no Pi login. |
| `gpu-tunnel` | Optional (profile `gpu-tunnel`). A chisel server that a GPU workstation dials into over HTTPS; the workstation's Ollama then appears as `gpu-tunnel.internal:11436`. |
| `postgres`, `neo4j` | Authoritative evidence storage and graph projection. |
| `ollama`, `ollama-init` | Serve and prepare `qwen3-embedding:0.6b`, and the worker model when it is not lent by a GPU workstation. |
| `caddy` / `Caddyfile.template` | Access code, frontend assets, graph reads, terminal HTTP and WebSocket proxying. `/` redirects to `/demo`. `conf.d/gpu-tunnel.caddy` adds the tunnel route, which carries its own secret instead of the access code. |
| `Dockerfile` | Builds the React frontend, Rust service, Pi integration, and ttyd runtime. |
| `lib.sh` | Shared source packaging and Azure transfer helpers, including frontend sources. |
| `run-local.sh` / `pi-settings.py` | Local orchestration and model selection from Pi's existing settings. |
| `provision.sh` / `cloud-init.yaml` | Optional Azure VM creation and first boot setup. |
| `deploy.sh` | Rebuilds and swaps the app on an existing Azure VM, with rollback if readiness fails. |
| `sync-config.sh` | Pushes the compose file, proxy, environment and tunnel settings to an existing VM without touching data, the Pi login or the running image. |
| `gpu-tunnel.sh` | Runs on the workstation: installs the chisel client, starts and stops the tunnel. |
| `make-seed.sh` / `reset-seed.sh` | Optional seed preparation and restoration. |

The hosted proxy preserves the branch's restricted API exposure: only graph reads are forwarded. Pi reaches retain, recall, namespace clear, and jobs over the internal network. The graph's Clear button is hidden in the demo; use `/memory-clear` inside Pi with typed confirmation.

## Validation

`frontend/e2e/demo.spec.ts` checks session switching, shared namespaces, safe URL encoding, reload behavior, and mobile layout without model calls. `deploy/azure/e2e.js` additionally exercises two actual Pi sessions and memory extraction against a running stack, including an initially empty graph. It calls your model provider.

```sh
(cd frontend && npm test && npm run test:e2e)
node deploy/azure/e2e.js http://127.0.0.1:18088 reviewer "$(cat ~/.local/state/sdp-hosted-local/access.code)"
```

## Azure

The optional provisioning command creates a VM and copies the authenticated Pi configuration (for the terminal only):

```sh
DNS_LABEL=my-unique-label ./deploy/azure/provision.sh
```

`DNS_LABEL` selects the page's Azure DNS name. The script supports `AZURE_RG`, `AZURE_LOC`, `AZURE_VM`, `VM_SIZE`, `PI_AGENT_DIR`, `PI_PROVIDER`, `PI_MODEL`, `GPU_TUNNEL`, `SEED_DUMP`, and `SDP_CLOUD_STATE`; its header documents defaults. It opens HTTPS and HTTP, and keeps SSH closed unless `SSH_FROM` is supplied. Generated credentials live in the cloud state directory. Provisioning has not been executed as part of this integration.

To update an existing VM, run `deploy/azure/deploy.sh`. Its image includes the React assets. The databases and credentials survive image updates; Compose and proxy configuration changes go to the VM with `deploy/azure/sync-config.sh`. The workflow and OIDC setup are described in `setup-oidc.sh` and `.github/workflows/deploy-azure.yml`.

## Lend a GPU to the Azure stack

The VM has two CPU cores and no GPU, and this subscription has no GPU quota, so the worker model is served by a workstation's GPU. The workstation dials out to the VM over HTTPS (port 443; nothing is opened on the workstation and no SSH is needed), and its Ollama appears inside the stack as `gpu-tunnel.internal:11436`.

One time, on the workstation (an Ollama on port 11436 that has the model; the Ollama binary is not part of this repository):

```sh
OLLAMA_HOST=127.0.0.1:11436 ollama serve &
OLLAMA_URL=http://127.0.0.1:11436 scripts/fetch-slm.sh
deploy/azure/gpu-tunnel.sh install
```

Then, to enable the tunnel on the VM and connect:

```sh
deploy/azure/sync-config.sh        # adds the gpu-tunnel service and the /_gpu route
deploy/azure/gpu-tunnel.sh up      # dial in; `status` and `down` do what they say
```

How it is protected: the tunnel server only accepts a client with the generated `GPU_TUNNEL_SECRET`, and its auth file allows exactly one reverse port (`11436`), so a stolen secret cannot reach any other service. Prompts and answers travel inside TLS to the VM. While the workstation is off or asleep, extraction, consolidation and reflection jobs fail with a connection error and are retried when it returns; the worker never falls back to another model. `GPU_TUNNEL=0` serves the model from the VM's own Ollama instead, which has to run it on 2 CPU cores and is far slower.
