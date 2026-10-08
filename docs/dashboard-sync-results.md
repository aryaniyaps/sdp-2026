# Dashboard project and Pi namespace acceptance — 2026-10-08

Implemented persisted project creation, project-specific defaults, and a versioned namespace binding for each dashboard/Pi session. The terminal launcher accepts validated project names and receives the binding UUID as its third argument. Namespace changes preserve the Pi process, apply during an active turn without waiting for spool delivery, and update the dashboard in both directions. Namespace switches and clears establish retention boundaries so previous messages are not copied or replayed.

Validation used isolated PostgreSQL databases (`dashboard_sync_test` for a live worker and `dashboard_fixture_test` for deterministic API fixtures), the local Neo4j service, and disposable project namespaces. Existing application namespaces were not cleared.

Passed:

- Frontend build/type check and 4 unit tests.
- 9 Playwright tests: six UI routing/layout checks, two dashboard-clear confirmation/error checks, and one real Pi RPC + live API + live graph acceptance test.
- Pi integration type check and 11 tests, including project spool isolation, namespace/clear retention boundaries, and immediate namespace handoff and rapid switching during an active turn.
- Rust build; database-backed project/session API test (persistence, validation, stale-write conflicts, session isolation, acknowledgement fencing).
- Existing PostgreSQL/Neo4j namespace-clear integration test, including preservation of another namespace.
- Launcher inside the hosted runtime image: new directory, per-project default, explicit namespace/session forwarding, and traversal rejection.
- Caddy configuration validation and HTTP checks: authenticated project/session operations work; unauthenticated requests are rejected; clear requires an authenticated POST and exact namespace confirmation; acknowledgement remains internal.

The live browser test launches the real Pi extension in RPC mode using the dashboard's actual session UUID. Only ttyd rendering is replaced; API calls and graph rendering are real. It verifies dashboard-to-Pi and Pi-to-dashboard namespace changes without replacing the terminal URL, wrong clear confirmation, successful clear, zero visible graph memories without page reload, another namespace remaining populated, and `/memory-status`. It also clears the other populated namespace with the dashboard button and verifies zero visible memories while preserving the terminal. It does not call a paid model provider. The screenshot is written to `frontend/test-results/dashboard-live.png`.

Two test issues were corrected during verification: a hidden empty-state text initially allowed a premature graph assertion, so acceptance now requires zero displayed memories and a visible empty state; the backend clear fixture conflicted with a running worker, so deterministic fixtures use a separate database without a worker.

Re-run live acceptance with an isolated engine and Neo4j:

```sh
cd frontend
LIVE_DASHBOARD=1 MEMORY_API_URL=http://127.0.0.1:18081 npm run test:e2e
```

Deployment must rebuild the engine/UI and Pi extension and apply the updated Caddy template and compose configuration together. Migration 0012 runs automatically at engine startup. The new `projects` volume preserves `/work` on subsequent container replacements; existing unmounted terminal work directories should be copied into that volume before replacing an existing terminal container. No hosted stack was rebuilt or replaced during these checks.

The dashboard Clear namespace dialog supports cancellation, rejects mismatched confirmation, shows worker conflicts for retry, and distinguishes queued graph cleanup from completed cleanup. The live test cleanup waits for Pi to exit before removing its session directory, avoiding a shutdown write race.

Immediate switching keeps an already-running turn’s evidence and cursor bound to its starting namespace, while the active namespace, Pi status, and subsequent memory operations change immediately. Stale automatic-recall responses from the previous namespace are not injected after a switch.

Starter project acceptance: migration 0012 was exercised against a newly created `dashboard_seed_acceptance` database. The catalog contains `payments-api`, `mobile-app`, and `scratch`, and each opens with its own `project:<name>` namespace. A browser test holds the catalog response and verifies all three starter projects are visible immediately, then confirms the live catalog adds custom projects and selects `payments-api`.

## Local 404 repair — 2026-10-08

The running native and browser APIs returned 404 for `/api/v2/projects`: their
binaries predated the dashboard routes, and the saved Caddy configuration also
lacked those routes. The current frontend/backend were rebuilt using installed
dependencies (Cargo offline), and the local runtime image was updated with the
binary and Pi extension. The proxy and terminal launcher were refreshed together.
Migration 0012 ran on startup. Existing terminal work files were backed up and
copied into the new persistent projects volume; database volumes were preserved.

Verified both direct APIs return the project catalog. Playwright against the real
proxy and ttyd loaded the dashboard, changed its namespace, observed Pi's sync
acknowledgement, and restored the original namespace with no failed HTTP responses
or page errors. Screenshot: `frontend/test-results/dashboard-404-fixed.png`.
All 14 script tests passed. Readiness now requires the proxied project API, and
online preparation refreshes managed proxy routes and the Pi launcher so a stale
proxy cannot silently survive an application update. This supersedes the earlier
statement that no hosted stack was rebuilt during the initial feature checks.
