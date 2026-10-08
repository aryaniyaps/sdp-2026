"""Exercise real general-purpose memory, preserving every receipt and namespace.

Run only against a local engine configured with the candidate model. No data is deleted.
"""
import argparse
import datetime as dt
import hashlib
import json
import re
import time
import uuid
from pathlib import Path

import requests


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--url", default="http://127.0.0.1:18080")
    p.add_argument("--out", type=Path, default=Path(__file__).parent / "data/research-v3/final-acceptance.json")
    p.add_argument("--timeout", type=int, default=1800)
    p.add_argument("--ollama-url", default="http://127.0.0.1:11434")
    a = p.parse_args()
    ns = "acceptance-general:" + str(uuid.uuid4())
    receipt = {"namespace": ns, "url": a.url, "started_at": dt.datetime.now(dt.timezone.utc).isoformat(), "steps": {}, "checks": {}}
    source_texts = []
    a.out.parent.mkdir(parents=True, exist_ok=True)

    def save():
        a.out.write_text(json.dumps(receipt, indent=2) + "\n")

    def request(method, route, **kwargs):
        r = requests.request(method, a.url + route, timeout=600, **kwargs)
        r.raise_for_status()
        return r.json()

    def deployment_identity():
        health = request("GET", "/healthz")
        model = health["worker_model"].removeprefix("ollama/")
        tags_response = requests.get(a.ollama_url + "/api/tags", timeout=30)
        tags_response.raise_for_status()
        names = {model, model if ":" in model else model + ":latest"}
        tag = next((tag for tag in tags_response.json()["models"] if tag["name"] in names), None)
        if tag is None:
            raise RuntimeError("Configured worker model was not found on the supplied Ollama server")
        show_response = requests.post(a.ollama_url + "/api/show", json={"model": model}, timeout=30)
        show_response.raise_for_status()
        show = show_response.json()
        blob = re.search(r"sha256-([0-9a-f]{64})", show.get("modelfile", ""))
        return {"health": health, "ollama_url": a.ollama_url, "ollama_model": tag["name"],
                "manifest_digest": tag["digest"], "served_gguf_sha256": blob[1] if blob else None,
                "details": show.get("details"), "parameters": show.get("parameters"),
                "template_sha256": hashlib.sha256(show.get("template", "").encode()).hexdigest(),
                "system_sha256": hashlib.sha256(show.get("system", "").encode()).hexdigest()}

    def wait_jobs(stage):
        deadline = time.monotonic() + a.timeout
        while time.monotonic() < deadline:
            rows = request("GET", "/api/v2/jobs", params={"namespace": ns})
            receipt["steps"][stage + "_jobs"] = rows
            save()
            if any(j["status"] == "failed" for j in rows):
                raise RuntimeError("A live worker job failed; see receipt")
            if rows and all(j["status"] == "succeeded" for j in rows):
                return
            time.sleep(3)
        raise TimeoutError("Live jobs did not finish within the recorded timeout")

    def retain(stage, content, timestamp):
        source_texts.append(content)
        receipt["steps"][stage + "_retain"] = request("POST", "/api/v2/retain", json={"namespace": ns, "session_id": "everyday-memory", "external_id": stage, "events": [{"role": "user", "content": content, "occurred_at": timestamp}]})
        save()
        wait_jobs(stage)
        receipt["steps"][stage + "_graph"] = request("GET", "/api/v2/graph", params={"namespace": ns})
        save()

    try:
        receipt["deployment_identity_before"] = deployment_identity()
        receipt["health"] = receipt["deployment_identity_before"]["health"]
        receipt["worker_model"] = receipt["health"].get("worker_model")
        retain("initial", "My name is Maya Sen. I live in Pune. I prefer vegetarian meals. I avoid meat when choosing restaurants. My sister is Leela Sen.", "2026-10-08T09:00:00Z")
        retain("correction", "I am Maya Sen. Correction: I moved from Pune to Chennai today. My current home city is Chennai, not Pune. My vegetarian meal preference has not changed.", "2026-10-08T10:00:00Z")
        graph = receipt["steps"]["correction_graph"]
        projection = request("GET", "/api/v2/graph/projection", params={"namespace": ns})
        receipt["projection"] = projection
        assertions = graph["assertions"]
        observations = [x for x in assertions if x["kind"] == "observation" and x["status"] == "active"]
        supports = [x for x in graph["edges"] if x["relation"] in ("supports", "derives")]
        receipt["checks"]["durable_assertions"] = len(assertions) >= 3
        receipt["checks"]["current_chennai"] = any("chennai" in x["statement"].lower() and x["status"] == "active" for x in assertions)
        receipt["checks"]["correction_history"] = any(x["status"] == "superseded" and "pune" in x["statement"].lower() for x in assertions)
        receipt["checks"]["old_city_not_active"] = not any(x["status"] == "active" and "pune" in x["statement"].lower() and "chennai" not in x["statement"].lower() for x in assertions)
        receipt["checks"]["sister_relationship"] = any(x["status"] == "active" and all(word in x["statement"].lower() for word in ("maya", "leela", "sister")) for x in assertions)
        receipt["checks"]["no_unsupported_geographic_addition"] = not any("india" in x["statement"].lower() for x in assertions)
        receipt["checks"]["derived_observation_with_two_supports"] = any(len({e["to_id"] for e in supports if e["from_id"] == x["id"]}) >= 2 for x in observations)
        receipt["checks"]["neo4j_projection"] = bool(projection.get("nodes")) and bool(projection.get("relationships"))
        for name, query in [("location", "Where does Maya Sen currently live?"), ("preference", "What meals does Maya Sen prefer?")]:
            for route in ("recall", "reflect"):
                value = request("POST", "/api/v2/" + route, json={"namespace": ns, "query": query, "reference_date": "2026-10-08T11:00:00Z"})
                receipt["steps"][name + "_" + route] = value
                if route == "recall":
                    results = value.get("results", [])
                    expected = "chennai" if name == "location" else "vegetarian"
                    receipt["checks"][name + "_recall_source_evidence"] = any(expected in hit.get("statement", "").lower() and any(source.get("quote") and any(source["quote"] in text for text in source_texts) for source in hit.get("sources", [])) for hit in results)
                if route == "reflect":
                    expected = "chennai" if name == "location" else "vegetarian"
                    receipt["checks"][name + "_cited_answer"] = expected in value.get("answer", "").lower() and bool(value.get("citations")) and not value.get("insufficient_evidence", True)
                    hits = {hit["id"]: hit for hit in value.get("recall", {}).get("results", [])}
                    receipt["checks"][name + "_citations_have_source_evidence"] = bool(value.get("citations")) and all(cid in hits and any(source.get("quote") and any(source["quote"] in text for text in source_texts) for source in hits[cid].get("sources", [])) for cid in value.get("citations", []))
                    if name == "location":
                        receipt["checks"]["answer_does_not_assert_old_city_current"] = not bool(re.search(r"(?:currently\s+)?(?:lives?|resides?|home\s+city\s+is)\s+(?:in\s+)?pune", value.get("answer", ""), re.I))
                save()
        receipt["deployment_identity_after"] = deployment_identity()
        receipt["checks"]["worker_identity_unchanged"] = receipt["deployment_identity_before"] == receipt["deployment_identity_after"]
        receipt["passed"] = all(receipt["checks"].values())
    except Exception as exc:
        receipt["error"] = str(exc)
        receipt["passed"] = False
    finally:
        receipt["finished_at"] = dt.datetime.now(dt.timezone.utc).isoformat()
        save()
    print(json.dumps({"receipt": str(a.out), "namespace": ns, "passed": receipt["passed"], "checks": receipt["checks"], "error": receipt.get("error")}, indent=2))
    raise SystemExit(0 if receipt["passed"] else 1)


if __name__ == "__main__":
    main()
