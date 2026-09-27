#!/usr/bin/env python3
"""Resume a frozen full experiment after subscription quota returns."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]


def api(base, path, post=False):
    request = urllib.request.Request(base.rstrip("/") + path, data=b"{}" if post else None,
                                     headers={"content-type": "application/json"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def probe(provider, model, thinking):
    command = ["pi", "--provider", provider, "--model", model, "--thinking", thinking,
               "--no-tools", "--no-extensions", "--no-skills", "--no-prompt-templates",
               "--no-session", "--mode", "json", "-p", 'Return exactly JSON {"ready":true}.']
    result = subprocess.run(command, cwd="/tmp", text=True, capture_output=True, timeout=180)
    messages = []
    for line in result.stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if event.get("type") == "message_end" and event.get("message", {}).get("role") == "assistant":
            messages.append(event["message"])
    if result.returncode or not messages or messages[-1].get("stopReason") in ("error", "aborted"):
        error = messages[-1].get("errorMessage", "No successful finalized answer") if messages else "No successful finalized answer"
        raise RuntimeError(f"{provider}/{model}: {error[:500]}. No jobs were retried.")
    text = "".join(part.get("text", "") for part in messages[-1].get("content", []) if part.get("type") == "text")
    if json.loads(text).get("ready") is not True:
        raise RuntimeError("Model availability probe did not confirm readiness. No jobs were retried.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", default="review-full-v3")
    parser.add_argument("--base", default="http://127.0.0.1:8080")
    parser.add_argument("--check", action="store_true", help="Inspect the recovery plan without model calls or mutations")
    parser.add_argument("--resume-demo", action="store_true", help="Also finish processing an existing three-session local demo")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.run):
        raise RuntimeError("Run ID must contain only letters, digits, underscores and hyphens")
    folder = ROOT / "benchmark/runs" / args.run
    manifest = json.loads((folder / "manifest.json").read_text())
    if manifest["development_run"] or len(manifest["question_ids"]) != 500 or (folder / "retired.json").exists():
        raise RuntimeError("Recovery requires a non-retired, frozen 500-question experiment")
    # The original runner verifies code/config, archive and dataset integrity.
    subprocess.run(["python3", "benchmark/run.py", "prepare", "--run", args.run], cwd=ROOT, check=True)
    if subprocess.check_output(["pi", "--version"], text=True).strip() != manifest["pi_version"]:
        raise RuntimeError("Pi version differs from the frozen experiment")
    config = manifest["config"]
    if config["worker_thinking"] != "medium":
        raise RuntimeError("The current Pi worker uses fixed medium thinking")
    health = api(args.base, "/healthz")
    if (health.get("worker_model") != f"pi/{config['worker_provider']}/{config['worker_model']}"
            or health.get("worker_concurrency") != config["worker_concurrency"]
            or health.get("embedding_model") != config["embedding_model"]
            or not health.get("database") or not health.get("embedder")):
        raise RuntimeError("Runtime health/models do not match the frozen experiment")
    query = "SELECT jsonb_build_object('id',id,'namespace',namespace,'error',error) FROM memory_jobs WHERE status='failed'"
    output = subprocess.check_output(["docker", "compose", "exec", "-T", "postgres", "psql", "-U", "memory",
                                      "-d", "memory_app", "-Atc", query], cwd=ROOT, text=True)
    allowed = {f"longmemeval:{args.run}:{qid}" for qid in manifest["question_ids"]}
    if args.resume_demo:
        for index in range(1, 4):
            artifact = ROOT / f"benchmark/runs/pi-coding-demo/session-{index}.jsonl"
            events = [json.loads(line) for line in artifact.read_text().splitlines() if line.startswith("{")]
            if not any(event.get("type") == "agent_end" for event in events):
                raise RuntimeError("--resume-demo requires three already-finished native sessions")
        allowed.add("demo:pi-duration-v1")
    failed = [json.loads(line) for line in output.splitlines() if line.startswith("{")]
    failed = [job for job in failed if job["namespace"] in allowed]
    if any("usage limit" not in (job.get("error") or "").lower() for job in failed):
        raise RuntimeError("A non-quota job failure requires inspection before resuming")
    print(json.dumps({"run": args.run, "questions": 500, "failed_quota_jobs": len(failed),
                      "archive_sha256": hashlib.sha256((folder / "source.tar.gz").read_bytes()).hexdigest(),
                      "check_only": args.check}), flush=True)
    if args.check:
        return
    for provider, model, thinking in sorted({(config["worker_provider"], config["worker_model"], config["worker_thinking"]),
                                             (config["reader_provider"], config["reader_model"], config["thinking"]),
                                             (config["judge_provider"], config["judge_model"], config["thinking"])}):
        probe(provider, model, thinking)
    for job in failed:
        api(args.base, f"/api/v2/jobs/{job['id']}/retry?namespace={urllib.parse.quote(job['namespace'])}", post=True)
    with (folder / "resume-demo.log").open("a") as log:
        demo = subprocess.Popen(["python3", "benchmark/demo/run.py"], cwd=ROOT, stdout=log, stderr=log) if args.resume_demo else None
        evaluation = subprocess.run(["python3", "benchmark/run.py", "all", "--run", args.run, "--base", args.base], cwd=ROOT)
        if evaluation.returncode:
            raise RuntimeError(f"Evaluation stopped with exit code {evaluation.returncode}; saved artifacts remain resumable")
        if demo is not None and demo.wait():
            raise RuntimeError("Demo final processing failed; inspect resume-demo.log")
    if args.resume_demo:
        subprocess.run(["python3", "benchmark/demo/verify.py"], cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
