# Readiness verification — 14 October 2026

The prepared local stack runs with the reviewed quote schema, contested-value correction repair and memory-provenance filtering. The source remains uncommitted; unrelated existing work is preserved.

Verified checks:
- Rust: 71 unit tests, 30 evidence-graph integration tests and 15 context/transport test cases passed in an isolated PostgreSQL database. Optional remote/model opt-in cases are not used as live acceptance evidence.
- Rust formatting and clippy with warnings denied passed.
- Pi: TypeScript check and all 13 integration/unit tests passed.
- Frontend: production build, 4 unit tests and 8 browser tests passed. The separate live Pi/dashboard namespace-and-clear acceptance test passed.
- Local startup: 15 tests passed and prepared-stack readiness passed.
- Python structured decoding: all 7 tests passed after synchronizing the review branches with the Rust schema.
- Video: 403.92-second 1920x1080 H.264 MP4 was fully decoded without errors, and representative frames were visually reviewed. Original Pi-only evidence and its recording receipt remain in output/video/raw.

Live regression: guide:live-ready-1791918973580 was populated exclusively through Pi messages and Pi's memory tool. The initial owner/database/observation remained identical after a new-session recall had been retained and all jobs finished. SQL inspection confirmed the recalled assistant answer has memory_derived=true. A fresh user correction after recall still made SQLite active and PostgreSQL superseded; the old PostgreSQL observation became stale. Owner Mira remained active. No new assertion was directly inserted for this rehearsal.

Causal lessons:
1. A source quote can be verbatim yet still be derivative. Keep recalled answers in the ledger, but do not promote them into independent fact sources.
2. Filtering model input must map source indices back to the original full episode. The long-window regression now interleaves excluded assistant evidence and verifies every stored quote points to the correct original event.
3. Schema changes need both Rust runtime and Python distillation parity checks. The Python helper initially expected the old INDICES placeholder; it now emits the same per-claim quote-bound branches as Rust.
4. A same-value correction must resolve a contested slot rather than stopping at deduplication.

The fresh presentation namespace in LIVE-DEMO.md is untouched. Old video/rehearsal namespaces are preserved as historical diagnostics, including the one that revealed recall feedback. Use the fresh URL for tomorrow, not an old recording namespace.

Final acceptance: another fresh Pi session correctly recalled Mira, SQLite and the superseded PostgreSQL report with citations. After its retention/extraction completed, all assertion IDs and statuses remained unchanged; zero outstanding jobs and ready=true. See receipt.json and final-recall.txt. The old stale observation is intentionally retained as history.
