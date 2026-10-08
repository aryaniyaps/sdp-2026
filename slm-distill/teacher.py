"""Client for the teacher model (gpt-6-sol on Azure OpenAI, Responses API).

The teacher is used offline, once, to write training data and to grade evaluation output. It is
never on the serving path. The key comes from the environment only:

    TEACHER_API_KEY        required
    TEACHER_BASE_URL       required, the OpenAI-compatible root of your Azure resource,
                           e.g. https://<resource>.services.ai.azure.com/openai/v1
    TEACHER_MODEL          default gpt-6-sol (the deployment name)

Every request is cached on disk by a hash of its body, so a re-run costs nothing and an
interrupted run resumes. Failures are raised; there is no fallback to another model.
"""

from __future__ import annotations

import hashlib
import json
import os
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_MODEL = "gpt-6-sol"


class TeacherError(RuntimeError):
    pass


class Teacher:
    def __init__(self, cache_dir: Path, usage_log: Path | None = None, effort: str = "low",
                 attempts: int = 12):
        key = os.environ.get("TEACHER_API_KEY")
        if not key:
            raise TeacherError("TEACHER_API_KEY is not set")
        self.key = key
        base = os.environ.get("TEACHER_BASE_URL")
        if not base:
            raise TeacherError("TEACHER_BASE_URL is not set")
        self.base = base.rstrip("/")
        self.model = os.environ.get("TEACHER_MODEL", DEFAULT_MODEL)
        self.cache_dir = Path(cache_dir)
        self.cache_dir.mkdir(parents=True, exist_ok=True)
        self.usage_log = usage_log
        self.effort = effort
        self.attempts = attempts
        self._lock = threading.Lock()
        self.calls = self.cached = self.input_tokens = self.output_tokens = self.reasoning_tokens = 0

    def _body(self, system: str, user: str, effort: str, max_output_tokens: int, json_mode: bool) -> dict:
        body = {
            "model": self.model,
            "input": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "max_output_tokens": max_output_tokens,
            "reasoning": {"effort": effort},
        }
        if json_mode:
            body["text"] = {"format": {"type": "json_object"}}
        return body

    def _post(self, body: dict) -> dict:
        data = json.dumps(body).encode()
        request = urllib.request.Request(
            f"{self.base}/responses", data=data,
            headers={"api-key": self.key, "content-type": "application/json"},
        )
        delay = 4.0
        last = ""
        for attempt in range(self.attempts):
            try:
                with urllib.request.urlopen(request, timeout=600) as response:
                    return json.load(response)
            except urllib.error.HTTPError as err:
                detail = err.read()[:300].decode("utf-8", "replace")
                last = f"HTTP {err.code}: {detail}"
                if err.code not in (408, 409, 429, 500, 502, 503, 504):
                    raise TeacherError(last) from err
                wait = float(err.headers.get("retry-after", delay))
            except (urllib.error.URLError, TimeoutError, ConnectionError) as err:
                last = f"{type(err).__name__}: {err}"
                wait = delay
            time.sleep(min(wait, 60))
            delay = min(delay * 2, 60)
        raise TeacherError(f"teacher request failed after {self.attempts} attempts: {last}")

    @staticmethod
    def _text(response: dict) -> str:
        if response.get("status") not in (None, "completed"):
            raise TeacherError(f"teacher response is {response.get('status')}: {json.dumps(response.get('incomplete_details'))}")
        parts = []
        for item in response.get("output", []):
            if item.get("type") == "message":
                for content in item.get("content", []):
                    if content.get("type") == "output_text":
                        parts.append(content["text"])
        if not parts:
            raise TeacherError("teacher returned no text")
        return "".join(parts)

    def complete(self, system: str, user: str, *, effort: str | None = None,
                 max_output_tokens: int = 24000, json_mode: bool = True, tag: str = "") -> str:
        effort = effort or self.effort
        body = self._body(system, user, effort, max_output_tokens, json_mode)
        digest = hashlib.sha256(json.dumps(body, sort_keys=True).encode()).hexdigest()
        path = self.cache_dir / digest[:2] / f"{digest}.json"
        if path.exists():
            with self._lock:
                self.cached += 1
            return json.loads(path.read_text())["text"]
        response = self._post(body)
        text = self._text(response)
        usage = response.get("usage", {})
        details = usage.get("output_tokens_details", {}) or {}
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(f".{os.getpid()}.{threading.get_ident()}.tmp")
        tmp.write_text(json.dumps({"text": text, "usage": usage, "tag": tag}))
        tmp.replace(path)
        with self._lock:
            self.calls += 1
            self.input_tokens += usage.get("input_tokens", 0)
            self.output_tokens += usage.get("output_tokens", 0)
            self.reasoning_tokens += details.get("reasoning_tokens", 0)
            if self.usage_log:
                with self.usage_log.open("a") as f:
                    f.write(json.dumps({"tag": tag, "model": self.model, "effort": effort, **{
                        "input_tokens": usage.get("input_tokens", 0),
                        "output_tokens": usage.get("output_tokens", 0),
                        "reasoning_tokens": details.get("reasoning_tokens", 0)}}) + "\n")
        return text

    def complete_json(self, system: str, user: str, **kwargs):
        text = self.complete(system, user, **kwargs)
        try:
            return json.loads(text)
        except json.JSONDecodeError as err:
            raise TeacherError(f"teacher reply is not JSON: {err}: {text[:200]!r}") from err

    def summary(self) -> str:
        return (f"teacher {self.model}: {self.calls} calls, {self.cached} cached, "
                f"{self.input_tokens:,} input, {self.output_tokens:,} output "
                f"({self.reasoning_tokens:,} reasoning) tokens")
