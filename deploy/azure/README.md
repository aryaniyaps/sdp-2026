# Pi session and live graph demo

The demo at `/demo` is a React page with a real Pi terminal beside the memory graph. Fresh installations start with `payments-api`, `mobile-app`, and `scratch`, with `payments-api` selected initially. These starter projects appear while the catalog loads; project controls enable once loading finishes. Create additional projects from the page; the catalog lives in PostgreSQL and terminal directories are created under `/work` in a persistent volume. Each project defaults to `project:<project name>`. Applying a namespace updates the corresponding running Pi session without restarting it; Pi's `/memory-namespace` command also updates the page and graph automatically. Pi checks for namespace changes every 250 ms, including during an active turn, and displays the active namespace in its status bar. The page shows pending/synced acknowledgement. Use the dashboard’s **Clear namespace** button or Pi’s `/memory-clear` command to delete the selected namespace with exact typed confirmation. The dashboard refreshes the graph after success; external changes also appear through polling.

The UI lives in `frontend/src/demo`: `DemoPage.tsx` owns the controls and panes, `session.ts` builds the terminal and graph URLs, and `demo.css` handles the layout. The graph retains its separate React page and canvas modules. There is no separate static landing page.

## Run locally

See the [local startup instructions in the root README](../../README.md#running-locally-including-offline).
Run `./deploy/azure/run-local.sh prepare` once while online; later
`./deploy/azure/run-local.sh` starts the saved stack without builds or downloads.
`check` verifies installed images, models and endpoints; `down` preserves memory.
Existing custom configuration is preserved during preparation.

## Services and modules

| Service or file | Responsibility |
| --- | --- |
| `term` | ttyd starts Pi with the memory extension for each browser connection. Repeated URL arguments pass the project directory, namespace and session binding UUID to `pi-session.sh`. |
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

The hosted proxy preserves the branch's restricted API exposure: graph reads, project/session controls, and confirmed namespace-clear POST requests are forwarded. Namespace synchronization uses versioned writes so stale clients cannot overwrite newer changes. Pi reaches retain, recall, and jobs over the internal network. The dashboard provides the clear confirmation dialog; the embedded graph stays read-only.

## Validation

`frontend/e2e/demo.spec.ts` checks project creation, project defaults, namespace switching without terminal restarts, reload behavior, and mobile layout without model calls. `frontend/e2e/dashboard-live.spec.ts` runs real Pi RPC commands against a live isolated service and checks the visible graph after clear (`LIVE_DASHBOARD=1 MEMORY_API_URL=http://127.0.0.1:18081 npm run test:e2e -- dashboard-live.spec.ts`). It substitutes only ttyd rendering and does not call a model provider. `deploy/azure/e2e.js` additionally exercises two actual Pi sessions and memory extraction against a running stack, including an initially empty graph. It calls your model provider.

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
