#!/usr/bin/env python3
"""Resolve and verify the Pi model that runs memory extraction.

With PI_PROVIDER and PI_MODEL both set, those are used. With neither set, Pi's own
default provider and model (from its settings.json) are used. Anything else is an
error. The choice is then checked with Pi: credentials are ready, the model exists,
and one small request succeeds with the same flags the worker uses.

Prints "<provider> <model>" on stdout when everything works. Progress and every
failure go to stderr with a non-zero exit, so the caller never starts a service
whose background jobs would all fail.
"""

import json
import os
import subprocess
import sys
from pathlib import Path


def fail(message):
    sys.exit(f"worker check failed: {message}")


def resolve():
    provider, model = os.environ.get("PI_PROVIDER"), os.environ.get("PI_MODEL")
    if bool(provider) != bool(model):
        fail("set both PI_PROVIDER and PI_MODEL, or neither to use Pi's default model")
    if provider:
        return provider, model, "environment"
    settings = (
        Path(os.environ.get("PI_CODING_AGENT_DIR", Path.home() / ".pi" / "agent"))
        / "settings.json"
    )
    try:
        data = json.loads(settings.read_text())
        return data["defaultProvider"], data["defaultModel"], str(settings)
    except (OSError, KeyError, ValueError) as error:
        fail(
            f"no PI_PROVIDER/PI_MODEL and no usable default in {settings} ({error!r}); see `pi --list-models`"
        )


def run(command, **kwargs):
    try:
        return subprocess.run(
            command, capture_output=True, text=True, timeout=180, **kwargs
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        fail(f"{command[0]} could not run: {error}")


def main():
    provider, model, source = resolve()
    print(f"worker model {provider}/{model} (from {source})", file=sys.stderr)

    auth = run(["pi", "auth", "check", "--provider", provider, "--json"])
    try:
        state = json.loads(auth.stdout)
    except ValueError:
        fail(
            f"`pi auth check` gave no JSON: {(auth.stdout + auth.stderr).strip()[:300]}"
        )
    if state.get("status") != "ready":
        fail(
            f"Pi provider {provider!r} is not ready ({state.get('reason', state.get('status'))}); run pi and use /login, or choose another provider"
        )

    listing = run(["pi", "--list-models", model]).stdout
    if not any(line.split()[:2] == [provider, model] for line in listing.splitlines()):
        fail(
            f"Pi does not list model {model!r} for provider {provider!r}; see `pi --list-models`"
        )

    probe = run(
        [
            "pi",
            "--provider",
            provider,
            "--model",
            model,
            "--thinking",
            "medium",
            "--no-tools",
            "--no-extensions",
            "--no-skills",
            "--no-prompt-templates",
            "--no-session",
            "--mode",
            "json",
            "--system-prompt",
            "Return one JSON object only.",
            "-p",
        ],
        input='Return {"ok":true} as one JSON object only.',
        cwd="/tmp",
        env={**os.environ, "MEMORY_WORKER": "1"},
    )
    reply = None
    for line in probe.stdout.splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if (
            event.get("type") == "message_end"
            and event.get("message", {}).get("role") == "assistant"
        ):
            reply = event["message"]
    if reply is None:
        fail(
            f"the probe request produced no assistant message: {(probe.stdout + probe.stderr).strip()[-300:]}"
        )
    if reply.get("stopReason") in ("error", "aborted"):
        fail(
            f"the probe request failed: {str(reply.get('errorMessage', 'no detail'))[:300]}"
        )
    print("worker probe succeeded", file=sys.stderr)
    print(provider, model)


if __name__ == "__main__":
    main()
