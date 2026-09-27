#!/usr/bin/env python3
"""Verify recorded native Pi sessions and the final API, without model judging."""
import json
import pathlib
import subprocess

OUTPUT = pathlib.Path(__file__).resolve().parents[1] / "runs/pi-coding-demo"


def main():
    sessions = []
    for index in range(1, 4):
        path = OUTPUT / f"session-{index}.jsonl"
        events = [json.loads(line) for line in path.read_text().splitlines() if line.startswith("{")]
        if not any(event.get("type") == "agent_end" for event in events):
            raise RuntimeError(f"session {index} did not finish")
        messages = [event["message"] for event in events if event.get("type") == "message_end"]
        memory = [message for message in messages if message.get("customType") == "sdp-memory-context"]
        tools = [event for event in events if event.get("type") == "tool_execution_end"]
        if not tools:
            raise RuntimeError(f"session {index} has no observed tool execution")
        if index > 1 and not memory:
            raise RuntimeError(f"session {index} received no repository memory")
        if index == 3 and not any("microsecond" in str(message.get("content", "")).lower() for message in memory):
            raise RuntimeError("session 3 did not receive the corrected API decision")
        sessions.append({"session": index, "artifact": path.name, "memory_messages": len(memory),
                         "tool_results": len(tools), "memory_context": [message["content"] for message in memory]})
    # Verify independently of the agent's own suite; zero discovered tests cannot pass.
    probe = '''from duration import parse_duration
for text, expected in [("1us", 1), ("2ms", 2000), ("3s", 3000000), (" 2ms ", 2000), ("0us", 0)]:
    actual = parse_duration(text)
    assert type(actual) is int and actual == expected, (text, actual, expected)
for text in ["-1us", "-2ms", "-3s", "garbage", "1m", "1ms junk", ""]:
    try:
        parse_duration(text)
    except ValueError:
        continue
    raise AssertionError(("malformed or negative input accepted", text))
print("12 independent API checks passed")
'''
    result = subprocess.run(["python3", "-c", probe], cwd=OUTPUT / "repository", capture_output=True, text=True)
    report = {"sessions": sessions, "independent_api_checks": 12, "exit_code": result.returncode,
              "stdout": result.stdout, "stderr": result.stderr,
              "scope": "Actual three-session integration demonstration; not a paired coding accuracy benchmark."}
    (OUTPUT / "demonstration-report.json").write_text(json.dumps(report, indent=2))
    if result.returncode:
        raise RuntimeError(result.stderr)
    print("Native Pi memory injection and final microsecond API verified across three sessions.")


if __name__ == "__main__":
    main()
