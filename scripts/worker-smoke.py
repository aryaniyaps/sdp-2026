#!/usr/bin/env python3
"""Live check of the memory worker model against a running service.

It retains two small coding-session episodes through the API (the second one changes a fact and
the first one contains a credential), waits for the worker jobs, and checks what recall returns:

  - the extract job and the consolidate job of each episode succeed (failures are printed),
  - recall finds the fact that was told,
  - the credential is not in any stored claim or recalled context,
  - after the correction the new value is the active one.

    scripts/worker-smoke.py                       # service on http://127.0.0.1:8080
    scripts/worker-smoke.py --url http://127.0.0.1:18080

It calls the real model, so run it with the service and the model server up. Exit status 1 means a
check failed, and each failure says which one.
"""

import argparse
import json
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone

SECRET = "sk-live-4eC39HqLyjWDarjtT1zdp7dcX"


def call(base, path, body=None, timeout=60):
    request = urllib.request.Request(
        base + path, data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.load(response)
    except urllib.error.HTTPError as err:
        raise SystemExit(f"{path}: HTTP {err.code}: {err.read()[:300].decode(errors='replace')}") from err


def event(role, content, at, **metadata):
    return {"role": role, "content": content, "occurred_at": at.strftime("%Y-%m-%dT%H:%M:%SZ"), "metadata": metadata}


def wait_for_jobs(base, namespace, expected_kinds, timeout):
    deadline = time.time() + timeout
    while time.time() < deadline:
        jobs = call(base, f"/api/v2/jobs?namespace={namespace}")
        failed = [j for j in jobs if j["status"] == "failed"]
        if failed:
            return jobs, failed
        done = {k: sum(1 for j in jobs if j["kind"] == k and j["status"] == "succeeded") for k in expected_kinds}
        if all(done[k] >= n for k, n in expected_kinds.items()):
            return jobs, []
        time.sleep(3)
    raise SystemExit(f"timed out after {timeout}s waiting for {expected_kinds}; jobs: "
                     + json.dumps([(j['kind'], j['status'], j.get('error')) for j in jobs]))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", default="http://127.0.0.1:8080")
    ap.add_argument("--timeout", type=int, default=420, help="seconds to wait for the worker jobs")
    args = ap.parse_args()
    base = args.url.rstrip("/")
    namespace = f"smoke:{uuid.uuid4().hex[:8]}"
    t0 = datetime.now(timezone.utc) - timedelta(hours=2)
    problems = []

    health = call(base, "/healthz")
    print(f"service worker model: {health.get('worker_model')}  database={health.get('database')}  embedder={health.get('embedder')}")
    if not health.get("database") or not health.get("embedder"):
        raise SystemExit("the service reports its database or embedder as not ready")

    first = [
        event("user", "Heads up: payments-api runs on Node 22 and we use pnpm as the package manager. Deploys go out on Thursdays at 17:00 IST.", t0),
        event("assistant", "Understood. I will use pnpm and run the tests now.", t0 + timedelta(seconds=5)),
        event("tool", "> payments-api@2.4.1 test\n> vitest run\n\n ✓ src/charge.test.ts (14 tests) 311ms\n FAIL src/refund.test.ts > refunds partially captured charge\nAssertionError: expected 4200 to equal 4150\n",
              t0 + timedelta(seconds=20), tool="bash", is_error=True, exit_code=1, input={"command": "pnpm test"}),
        event("user", f"The refund bug is rounding: Stripe amounts are in minor units, so use Math.round. My live key is {SECRET} if you need it.", t0 + timedelta(seconds=60)),
    ]
    r1 = call(base, "/api/v2/retain", {"namespace": namespace, "session_id": "smoke-1", "external_id": "smoke-1", "events": first, "metadata": {}})
    print(f"retained episode 1 in {namespace}: job {r1['job_id']}")
    jobs, failed = wait_for_jobs(base, namespace, {"extract": 1, "consolidate": 1}, args.timeout)
    for j in failed:
        problems.append(f"{j['kind']} job failed: {j.get('error')}")
    if not failed:
        print("episode 1: extract and consolidate jobs succeeded")

    second = [
        event("user", "Update: we moved payments-api back to npm last night, pnpm caused trouble in CI. Please never run pnpm here again.", t0 + timedelta(hours=20)),
        event("assistant", "Noted: payments-api uses npm again.", t0 + timedelta(hours=20, seconds=5)),
    ]
    if not failed:
        call(base, "/api/v2/retain", {"namespace": namespace, "session_id": "smoke-2", "external_id": "smoke-2", "events": second, "metadata": {}})
        jobs, failed = wait_for_jobs(base, namespace, {"extract": 2, "consolidate": 2}, args.timeout)
        for j in failed:
            problems.append(f"{j['kind']} job failed: {j.get('error')}")
        if not failed:
            print("episode 2: extract and consolidate jobs succeeded")

    status = call(base, f"/api/v2/status?namespace={namespace}")
    print("status:", json.dumps({k: v for k, v in status.items() if k in ("assertions", "entities", "pending_jobs", "failed_jobs")}))
    recalled = call(base, "/api/v2/recall", {"namespace": namespace, "query": "Which package manager does payments-api use?", "max_tokens": 2000, "temporal": False}, timeout=120)
    context = recalled["context"]
    print("--- recalled context ---")
    print(context[:2500])
    print("------------------------")
    if "pnpm" not in context and "npm" not in context:
        problems.append("recall returned nothing about the package manager")
    if SECRET in context or SECRET[:12] in context:
        problems.append("the credential appears in the recalled context")
    graph = call(base, f"/api/v2/graph/projection?namespace={namespace}")
    if SECRET[:12] in json.dumps(graph):
        problems.append("the credential appears in the graph projection")
    npm_active = any("; active;" in l for l in context.splitlines() if l.startswith("[") and ("npm" in l.lower()) and "pnpm" not in l.lower())
    if not npm_active:
        problems.append("after the correction, no active assertion says npm")
    if problems:
        print("\nFAILED checks:")
        for p in problems:
            print(" -", p)
        sys.exit(1)
    print("\nall checks passed")


if __name__ == "__main__":
    main()
