# Pi in a browser, next to the live memory graph

One page: a real Pi terminal on the left, the memory graph of the same user on the right. Tell Pi a fact in one session, open a session in the other directory, ask about it, and watch the graph fill in as the worker extracts the fact. It runs on one Azure VM behind HTTPS and one access code, and the same stack runs on your own machine.

## What runs

| Service | What it is |
|---|---|
| `caddy` | HTTPS with an automatic certificate and one access code (user `reviewer`). Only the page, the terminal and the read-only graph routes are reachable. The memory API (retain, recall, clear, jobs) is not exposed. |
| `term` | ttyd, starting one fresh Pi for each browser connection, in `/work/payments-api` or `/work/mobile-app`, with the memory extension. |
| `engine` | The memory service. Its extraction worker is Pi, with the same login as the terminal. |
| `postgres`, `neo4j` | Storage. Neo4j holds the graph projection. |
| `ollama` | Serves `qwen3-embedding:0.6b` for embeddings, on the CPU. `ollama-init` pulls the model once. |

`term` and `engine` are one image, built from `Dockerfile` here. Files in this folder:

| File | Purpose |
|---|---|
| `web/index.html` | The page: two iframes, a button for each directory, an optional link to a recording. |
| `Caddyfile.template` | The proxy. `__SITE_ADDRESS__` and `__ACCESS_HASH__` are filled in when the stack is set up. |
| `docker-compose.yml` | The stack. |
| `pi-session.sh` | What ttyd starts for each connection: Pi with the memory extension in one of three fixed directories. `PI_TOOLS=off` in the stack's `.env` removes Pi's shell and file tools. |
| `run-local.sh` | The whole stack on this machine. |
| `provision.sh` | Creates the VM and the stack on Azure from nothing. |
| `deploy.sh`, `lib.sh` | Update the app on an existing VM. `lib.sh` is the shared helper. |
| `setup-oidc.sh` | One-time setup so the GitHub workflow can run `deploy.sh`. |
| `cloud-init.yaml` | Installs Docker and adds swap on the VM's first boot. |
| `make-seed.sh`, `reset-seed.sh` | Make a seed memory from the local database, and put a stack back to it. |
| `e2e.js`, `screenshot.js` | Check the page with a real headless Chrome. |

## Try it on your machine

```sh
deploy/azure/run-local.sh
```

It needs Docker Compose 2.24 or newer, openssl and a Pi that is logged in (`~/.pi/agent/auth.json`). It builds the image (several minutes the first time, it compiles the service), starts the stack with its own databases, pulls the embedding model and prints the address, `http://127.0.0.1:18088/`, and the access code. `run-local.sh down` stops it, `run-local.sh destroy` removes it and its memory. It does not touch the memory of `scripts/run-memory.sh`.

To start from a prepared memory instead of an empty one, make a seed from your local database and pass it in:

```sh
deploy/azure/make-seed.sh /tmp/seed.dump           # copies memory_app to a scratch database and keeps only user:dev
SEED_DUMP=/tmp/seed.dump deploy/azure/run-local.sh
```

To check the page with a real browser, including two real Pi sessions (this calls your model provider a few times):

```sh
node deploy/azure/e2e.js http://127.0.0.1:18088 reviewer "$(cat ~/.local/state/sdp-hosted-local/access.code)"
```

## Put it on Azure

```sh
az login
DNS_LABEL=my-unique-label SEED_DUMP=/tmp/seed.dump deploy/azure/provision.sh
```

`DNS_LABEL` must be unused in the region: the page becomes `https://<label>.eastus.cloudapp.azure.com/`. The script's header lists the other settings (resource group, location, VM size, Pi provider and model, a link for the recording, a smaller or no seed). It:

1. Creates the resource group, a network group that allows ports 80 and 443 only, and an Ubuntu 24.04 VM (Standard_D2s_v4, 2 vCPU and 8 GB) whose first boot installs Docker.
2. Generates the database passwords and the access code in `~/.sdp-cloud/secrets.env` (mode 600) and hashes the code for the proxy.
3. Sends the stack's files to `/opt/sdp`, including your Pi login (`~/.pi/agent/auth.json`) and the seed, starts the databases and pulls the embedding model.
4. Builds the app image on the VM with `deploy.sh` and starts it, restores the seed, starts the proxy and prints the address.

Nothing here uses SSH, and port 22 stays closed (set `SSH_FROM` to a CIDR to open it). Files travel through `az vm run-command`, which limits a script to 256 KB, so a file goes over in parts of 150 KB and each part takes about 12 seconds: a 5 MB seed takes several minutes and the script refuses a seed over 20 MB. The Pi login and the passwords travel inside those scripts, so the last step deletes the copies the Azure agent keeps on the VM. Anyone who is root on the VM can read the Pi login anyway, since the stack needs it, so use a provider login you are willing to put on a server.

The VM costs about 0.10 USD an hour while it runs, plus about 10 USD a month for the disk and 4 USD for the address. Stop paying for compute with `az vm deallocate -g <group> -n <vm>` and start again with `az vm start`. `az group delete -n <group>` removes everything.

Status of this script: the commands are the ones that created the current VM, and its uploads, file modes, proxy template, compose file and seed restore were each run and checked (on the VM and in `run-local.sh`). It has not been run in one piece against an empty subscription, and two parts differ from how the current VM was made and were not run as written: the network group is attached when the VM is created (`--nsg`; it was attached afterwards), and the wait for the first boot. A region can lack a VM size for a subscription; if `az vm create` says so, set `VM_SIZE`.

## Update the app

```sh
deploy/azure/deploy.sh
```

Builds the image from this checkout on the VM, swaps `engine` and `term` to it, waits for the engine to answer and puts the previous image back if it does not. A failed build changes nothing. The database, the graph, the proxy and the Pi login stay as they are, so the stored memory survives. Migrations run when the engine starts, and a migration that has run is not undone by the rollback, because an older engine refuses a database that is newer than itself. The three newest images are kept.

`.github/workflows/deploy-azure.yml` runs this on every push to `main` that touches the service or this folder. It signs in with OIDC, so no password is stored in GitHub. One-time setup:

1. Someone who can create app registrations in the tenant and assign roles runs `deploy/azure/setup-oidc.sh`. It creates the app, trusts the `azure` environment of the repository and grants Virtual Machine Contributor on the one VM. It prints three values.
2. A repository admin creates the `azure` environment (Settings, Environments) and stores them as its secrets: `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`. Required reviewers can be added to that environment if a deploy should wait for approval.
3. Run the workflow once from the Actions tab to check it.

The workflow does not run the tests, so a change that compiles but misbehaves is still deployed. Changes to `docker-compose.yml`, the proxy or `web/` are not part of a deploy; make them on the VM in `/opt/sdp`.

## Everyday commands

```sh
# put the memory back to the seed (removes what was told since); needs the seed in /opt/sdp
az vm run-command invoke -g <group> -n <vm> --command-id RunShellScript \
  --scripts 'cd /opt/sdp && ./reset-seed.sh && echo done'

# see the containers
az vm run-command invoke -g <group> -n <vm> --command-id RunShellScript \
  --scripts 'cd /opt/sdp && docker compose ps' --query 'value[0].message' -o tsv
```

`screenshot.js <base url> <user> <password> <out.png>` writes a picture of the page once Pi has started and the graph has loaded.

## Things to know

- The graph page has a Clear button. On the hosted page it fails, because the proxy does not forward `POST /api/v2/graph/clear`; that is on purpose, so a visitor cannot wipe the memory. Use `reset-seed.sh`.
- Cold recall on the 2 vCPU VM took about 0.6 to 1.7 seconds in a check, and a repeated question 40 ms: the query embedding runs on the CPU.
- SSH was not usable from the network this was built on, even from the allowed address, which is why everything goes through `az vm run-command`.
