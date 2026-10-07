#!/usr/bin/env python3
"""Cross-session, cross-directory memory check driven through Pi's RPC mode.

Every Pi session is a separate `pi --mode rpc` process with its own working
directory and its own session directory, so nothing is shared except the memory
service. The script fails loudly: any missing injection, missing fact, extension
error, or failed memory job is a non-zero exit with the evidence printed.

Scenarios
  1. Tell Pi a unique fact in directory A.
  2. A fresh session in directory B asks about it right away, before extraction
     has necessarily finished (raw evidence must already be recallable).
  3. Control: a fresh session with the extension disabled must NOT know it.
  4. After extraction completes, a fresh session in directory C asks again.
  5. Correct the fact in directory D, then a fresh session in directory E asks
     for the current value.

Requires: the memory service running (scripts/run-memory.sh), and a Pi install
authenticated for the chosen provider. The answering model is billed by that
provider; each scenario is one short exchange.
"""

import argparse
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXTENSION = ROOT / "integrations" / "pi" / "extension.ts"
CONTEXT_TYPE = "sdp-memory-context"


class Failure(Exception):
    pass


def http_json(url, body=None, timeout=30):
    data = None if body is None else json.dumps(body).encode()
    request = urllib.request.Request(
        url, data=data, headers={"content-type": "application/json"}
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise Failure(
            f"{url} returned HTTP {error.code}: {error.read().decode()[:400]}"
        ) from error
    except OSError as error:
        raise Failure(f"{url} is unreachable: {error}") from error


class PiSession:
    """One Pi process speaking the RPC JSONL protocol on stdin/stdout."""

    def __init__(self, label, cwd, args, env):
        self.label = label
        self.cwd = cwd
        self.events = []
        self.stderr_lines = []
        self._queue = queue.Queue()
        command = [
            "pi",
            "--mode",
            "rpc",
            "--session-dir",
            str(args.session_root / label),
        ]
        if args.provider:
            command += ["--provider", args.provider]
        if args.model:
            command += ["--model", args.model]
        command += ["--thinking", args.thinking]
        if not args.realistic:
            command += [
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--no-context-files",
            ]
        command += ["--no-builtin-tools"]
        if self.label.startswith("control"):
            pass
        else:
            command += ["-e", str(EXTENSION)]
        self.process = subprocess.Popen(
            command,
            cwd=cwd,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=False,
        )
        threading.Thread(target=self._read_stdout, daemon=True).start()
        threading.Thread(target=self._read_stderr, daemon=True).start()

    def _read_stdout(self):
        # RPC framing: records end only at LF. Do not use universal-newline readers.
        for raw in self.process.stdout:
            line = raw.rstrip(b"\n").rstrip(b"\r")
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                self.stderr_lines.append(f"[non-JSON stdout] {line[:200]!r}")
                continue
            self.events.append(event)
            self._queue.put(event)

    def _read_stderr(self):
        for raw in self.process.stderr:
            self.stderr_lines.append(raw.decode(errors="replace").rstrip())

    def send(self, command):
        self.process.stdin.write((json.dumps(command) + "\n").encode())
        self.process.stdin.flush()

    def wait_for(self, predicate, timeout, what):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                event = self._queue.get(timeout=1)
            except queue.Empty:
                if self.process.poll() is not None:
                    raise Failure(
                        f"[{self.label}] Pi exited ({self.process.returncode}) while waiting for {what}\n{self.tail_stderr()}"
                    )
                continue
            if predicate(event):
                return event
        raise Failure(
            f"[{self.label}] timed out after {timeout}s waiting for {what}\n{self.tail_stderr()}"
        )

    def tail_stderr(self):
        return "\n".join(self.stderr_lines[-15:])

    def prompt(self, text, timeout):
        request_id = str(uuid.uuid4())
        self.send({"id": request_id, "type": "prompt", "message": text})
        response = self.wait_for(
            lambda e: e.get("type") == "response" and e.get("id") == request_id,
            60,
            "prompt acceptance",
        )
        if not response.get("success"):
            raise Failure(f"[{self.label}] prompt rejected: {response}")
        self.wait_for(
            lambda e: e.get("type") == "agent_settled", timeout, "agent_settled"
        )

    def close(self):
        # Closing stdin lets Pi run session_shutdown, which awaits the extension's final flush.
        try:
            self.process.stdin.close()
        except OSError:
            pass
        try:
            self.process.wait(timeout=60)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=10)
            raise Failure(f"[{self.label}] Pi did not exit after stdin closed")

    def answer(self):
        text = ""
        for event in self.events:
            if (
                event.get("type") == "message_end"
                and event.get("message", {}).get("role") == "assistant"
            ):
                parts = event["message"].get("content", [])
                joined = "".join(
                    p.get("text", "") for p in parts if p.get("type") == "text"
                )
                if joined:
                    text = joined
        return text

    def injected(self):
        found = []
        for event in self.events:
            message = (
                event.get("message") if event.get("type") == "message_end" else None
            )
            if message and message.get("customType") == CONTEXT_TYPE:
                found.append(message)
        return found

    def problems(self):
        out = []
        for event in self.events:
            if event.get("type") == "extension_error":
                out.append(f"extension_error: {json.dumps(event)[:400]}")
            if (
                event.get("type") == "extension_ui_request"
                and event.get("method") == "setStatus"
                and event.get("statusKey") == "memory"
            ):
                out.append(f"memory status: {event.get('statusText')}")
        out += [line for line in self.stderr_lines if line.startswith("[memory]")]
        return out


def wait_ready(base, namespace, timeout):
    deadline = time.time() + timeout
    status = {}
    while time.time() < deadline:
        status = http_json(
            f"{base}/api/v2/status?namespace={urllib.request.quote(namespace)}"
        )
        failed = [g for g in status["jobs"] if g["status"] == "failed"]
        if failed:
            jobs = http_json(
                f"{base}/api/v2/jobs?namespace={urllib.request.quote(namespace)}"
            )
            errors = [j.get("error") for j in jobs if j["status"] == "failed"]
            raise Failure(
                f"memory jobs failed for {namespace}: {failed}; errors: {errors[:3]}"
            )
        if status["ready"]:
            return status
        time.sleep(2)
    raise Failure(
        f"namespace {namespace} was not ready after {timeout}s: {json.dumps(status)[:600]}"
    )


def check(report, name, ok, detail=""):
    report.append((name, ok, detail))
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}" + (f": {detail}" if detail else ""))
    return ok


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "--memory-url", default=os.environ.get("MEMORY_URL", "http://127.0.0.1:8080")
    )
    parser.add_argument(
        "--provider",
        help="Pi provider for the answering model (default: Pi's own default)",
    )
    parser.add_argument(
        "--model", help="Pi model for the answering model (default: Pi's own default)"
    )
    parser.add_argument("--thinking", default="off")
    parser.add_argument(
        "--timeout", type=int, default=240, help="seconds per agent run"
    )
    parser.add_argument(
        "--extraction-timeout",
        type=int,
        default=300,
        help="seconds to wait for memory extraction",
    )
    parser.add_argument(
        "--defaults",
        action="store_true",
        help="use the extension's real default namespace and spool instead of an isolated test namespace",
    )
    parser.add_argument(
        "--realistic",
        action="store_true",
        help="keep the user's own Pi extensions, skills and context files",
    )
    parser.add_argument(
        "--keep", action="store_true", help="keep temporary directories"
    )
    args = parser.parse_args()

    if shutil.which("pi") is None:
        sys.exit("pi is not on PATH")
    base = args.memory_url.rstrip("/")
    health = http_json(f"{base}/healthz")
    print(f"memory service: {json.dumps(health)[:300]}")

    run = uuid.uuid4().hex[:8]
    work = Path(tempfile.mkdtemp(prefix=f"pi-e2e-{run}-"))
    args.session_root = work / "pi-sessions"
    dirs = {name: work / name for name in "ABCDE"}
    for name, path in dirs.items():
        path.mkdir()
    # A is a git repository, the rest are plain directories: no shared repository root.
    subprocess.run(["git", "init", "-q"], cwd=dirs["A"], check=True)

    env = dict(os.environ, MEMORY_URL=base)
    namespace = None
    if not args.defaults:
        namespace = f"e2e:{run}"
        env["MEMORY_NAMESPACE"] = namespace
        env["MEMORY_SPOOL"] = str(work / "spool")
    codename = f"Marmalade-{uuid.uuid4().hex[:6]}"
    report = []
    sessions = []
    started = time.time()

    def session(label, directory, extra_env=None):
        s = PiSession(label, directory, args, dict(env, **(extra_env or {})))
        sessions.append(s)
        return s

    try:
        print(f"\n1. Tell Pi a fact in {dirs['A']} (codename {codename})")
        s1 = session("s1-tell", dirs["A"])
        s1.prompt(
            f"Remember this for future sessions: my side project is codenamed {codename}, it is a Rust command line tool, and I always deploy it on Fridays. Reply with just the word noted.",
            args.timeout,
        )
        s1.close()
        check(
            report,
            "session 1 had no memory or extension problems",
            not s1.problems(),
            "; ".join(s1.problems()),
        )

        print(f"\n2. Fresh session in {dirs['B']} asks immediately")
        s2 = session("s2-ask-immediately", dirs["B"])
        s2.prompt(
            "What is my side project's codename, and which day do I deploy it?",
            args.timeout,
        )
        s2.close()
        injected = s2.injected()
        if injected:
            namespace = namespace or injected[0].get("details", {}).get("namespace")
        check(
            report, "memory context was injected into the new session", bool(injected)
        )
        check(
            report,
            "injected context contains the codename",
            any(codename in json.dumps(m) for m in injected),
        )
        check(
            report,
            "answer contains the codename",
            codename.lower() in s2.answer().lower(),
            s2.answer()[:160].replace("\n", " "),
        )
        check(report, "answer says Friday", "friday" in s2.answer().lower())
        check(
            report,
            "session 2 had no memory or extension problems",
            not s2.problems(),
            "; ".join(s2.problems()),
        )

        print("\n3. Control: same question with the memory extension disabled")
        s3 = session("control-no-memory", dirs["C"])
        s3.prompt(
            "What is my side project's codename, and which day do I deploy it?",
            args.timeout,
        )
        s3.close()
        check(
            report,
            "control does not know the codename",
            codename.lower() not in s3.answer().lower(),
            s3.answer()[:160].replace("\n", " "),
        )

        print("\n4. Wait for extraction, then ask again from another directory")
        if not namespace:
            raise Failure("could not determine the namespace the extension used")
        status = wait_ready(base, namespace, args.extraction_timeout)
        check(
            report,
            "all memory jobs succeeded",
            status["ready"] and status["outstanding_jobs"] == 0,
            f"namespace {namespace}",
        )
        s4 = session("s4-ask-after-extraction", dirs["C"])
        s4.prompt("What is my side project's codename?", args.timeout)
        s4.close()
        check(
            report,
            "answer after extraction contains the codename",
            codename.lower() in s4.answer().lower(),
            s4.answer()[:160].replace("\n", " "),
        )
        graph = http_json(
            f"{base}/api/v2/graph?namespace={urllib.request.quote(namespace)}"
        )
        statements = [a["statement"] for a in graph["assertions"]]
        check(
            report,
            "extraction produced assertions that mention the codename",
            any(codename.lower() in s.lower() for s in statements),
            f"{len(statements)} assertions",
        )

        print(
            "\n5. Correct the fact in one directory, ask for the current value in another"
        )
        s5 = session("s5-correct", dirs["D"])
        s5.prompt(
            "Correction for my side project: I no longer deploy on Fridays, I deploy on Tuesdays now. Reply with just the word updated.",
            args.timeout,
        )
        s5.close()
        wait_ready(base, namespace, args.extraction_timeout)
        s6 = session("s6-ask-current", dirs["E"])
        s6.prompt(
            "Which day do I deploy my side project now? Answer in one sentence.",
            args.timeout,
        )
        s6.close()
        answer = s6.answer().lower()
        check(
            report,
            "current answer says Tuesday",
            "tuesday" in answer,
            s6.answer()[:200].replace("\n", " "),
        )
        check(
            report,
            "current answer does not present Friday as current",
            "friday" not in answer
            or any(
                w in answer
                for w in (
                    "previous",
                    "used to",
                    "changed",
                    "switched",
                    "formerly",
                    "earlier",
                    "no longer",
                    "instead of",
                )
            ),
            s6.answer()[:200].replace("\n", " "),
        )
        for s in (s5, s6):
            check(
                report,
                f"{s.label} had no memory or extension problems",
                not s.problems(),
                "; ".join(s.problems()),
            )
    except Failure as failure:
        check(
            report,
            "run completed without an infrastructure failure",
            False,
            str(failure),
        )
    finally:
        for s in sessions:
            if s.process.poll() is None:
                s.process.kill()
        print(
            f"\nelapsed {time.time() - started:.0f}s; namespace {namespace}; working directory {work}"
        )
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    failed = [name for name, ok, _ in report if not ok]
    print(f"\n{len(report) - len(failed)}/{len(report)} checks passed")
    if failed:
        print("FAILED: " + "; ".join(failed))
        sys.exit(1)


if __name__ == "__main__":
    main()
