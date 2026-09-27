# Native Pi integration demonstration

Verified locally on 2026-10-05 using Pi 1.0.2, provider `openai`, model
`gpt-5.6-sol`, and the repository's native Pi extension.

Three actual, fresh Pi sessions operated on a disposable duration-parser repository:

| Session | Task | Observed tool results | Injected memory messages |
|---|---|---:|---:|
| 1 | Implement standard-library duration parsing returning milliseconds | 9 | 0 |
| 2 | Explicitly change the API to integer microseconds, add `us`, preserve negative-input rejection | 8 | 1 |
| 3 | Fix whitespace and malformed-input behavior while preserving the remembered API decision | 11 | 1 |

The finalized session-three JSONL contains a native `sdp-memory-context` message
with the corrected microsecond decision. The agent's final repository passes seven
unit tests. The separate verifier also passes twelve independent checks covering
`us`/`ms`/`s` scaling, whitespace, integer return values, zero, negative durations
and malformed strings. The verifier rejects unfinished sessions, missing tool
execution, absent memory injection and a missing microsecond correction.

Run the demonstration and verifier:

```sh
python3 benchmark/demo/run.py
python3 benchmark/demo/verify.py
```

Raw artifacts are local under `benchmark/runs/pi-coding-demo/`:
`session-{1,2,3}.jsonl`, session stderr, the disposable repository, status snapshots,
the durable spool and `demonstration-report.json`. The runner waits for retained
evidence processing before writing its final `verification.json`; final-session
memory processing was still queued when the independent API check completed.

This proves the integration ran across sessions and delivered the changed decision
to a later coding task. It does not establish a causal accuracy improvement: there
is no paired no-memory coding control. The separate frozen LongMemEval experiment
provides the planned controlled memory comparison and is still in progress.
