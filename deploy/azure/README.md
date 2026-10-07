# Deploying to the Azure VM

The hosted copy (Pi in a browser terminal next to the live memory graph) runs on one VM with Docker
Compose in `/opt/sdp`. Pushing service source to `main` rebuilds it automatically.

## What a deploy does

`deploy.sh` is the whole procedure, and the GitHub workflow only logs in and runs it:

1. Packs the service source (`src`, `migrations`, `integrations/pi`, the Cargo files) and sends it to the VM with
   `az vm run-command`. No SSH is needed, so the VM firewall can stay closed to the internet.
2. Builds `sdp-memory-app:<commit>` on the VM from `deploy/azure/Dockerfile`. A failed build changes nothing.
3. Points the `engine` and `term` services at the new image and waits for the engine to answer.
4. If the engine is not healthy within two minutes, puts the previous image back and fails the run.
5. Keeps the three newest images and removes older ones.

The database, the graph, the proxy and the Pi login stay as they are, so the stored memory survives a deploy.
Migrations run when the engine starts. A migration that has run cannot be undone by the rollback in step 4,
because an older engine refuses a database that is newer than itself.

Run it by hand with `az login` done and a checkout of the commit to deploy:

```bash
deploy/azure/deploy.sh
```

A checkout with uncommitted changes is deployed too, and its image tag ends in `-dirty`.

## One-time setup for the workflow

The workflow signs in to Azure with OIDC, so no password or key is stored in GitHub.

1. Someone who can create app registrations in the tenant and assign roles runs `deploy/azure/setup-oidc.sh`.
   It creates the app, trusts the `azure` environment of this repository, and grants Virtual Machine Contributor
   on the one VM only. It prints three values.
2. A repository admin creates the `azure` environment (Settings, Environments) and stores them as its secrets:
   `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`. Required reviewers can be added to that
   environment if a deploy should wait for an approval.
3. Run the workflow once from the Actions tab (Deploy to Azure, Run workflow) to check it.

## Limits

- The workflow runs on pushes to `main` that touch the service, `deploy/azure/` or the workflow. It does not run
  the tests, so a change that compiles but misbehaves is still deployed.
- `docker-compose.yml`, the proxy configuration and the access code live on the VM and are not part of a deploy.
- The VM has no BuildKit, so the image builds without a layer cache for the Rust step (about 8 minutes).
