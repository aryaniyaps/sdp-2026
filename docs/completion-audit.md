# Completion audit — work remains

Status observed on 2026-10-05. The authoritative scope is
`project-requirements.json`. This audit does **not** declare the goal complete.

The user subsequently delegated benchmark execution to a friend. The handoff is
[`benchmark/README.md`](../benchmark/README.md); the partial run stopped at the
subscription-sharing usage limit. Final empirical completion remains the team's
responsibility, and the report is explicitly incomplete.

| Requirement group | Evidence inspected | Current conclusion |
|---|---|---|
| Storage and temporal graph | Migrations 0003–0010, `knowledge_store.rs`, PostgreSQL integration tests, live correction/provenance UI | Implemented; lifecycle, lease recovery, bank serialization and retroactive correction tests pass |
| Evidence and observations | Exact-source validation, extraction and consolidation repair loops, support checks, invalidation tests, real demo consolidation retry | Implemented; the real invalid-support response was repaired and committed successfully |
| Retrieval and reflection | `v2.rs`, graph traversal limits and authoritative validation, lifecycle tests, foreign-citation test, native browser recall/provenance check | Implemented and locally exercised; overall retrieval quality still requires the full experiment |
| Native Pi harness | TypeScript checks, three integration tests, all three actual session JSONLs, `demonstration-report.json`, seven parser unit tests and twelve independent checks | Three sessions and memory injection verified; final-session evidence processing still pending at this observation |
| Dataset and protocol | Checksummed 500-question dataset, 23,867 histories, pinned rubric, frozen manifest/source archive, five passing protocol tests | Protocol implemented; repeated session IDs have distinct episode identities without changing scoring provenance |
| Four primary conditions | `review-full-v3` frozen manifest, live ingestion and full-history reader processes, saved answer files | In progress; complete primary results are missing |
| Repetitions and ablations | Fixed 100-question sample and runner logic; report gates | Implemented but not yet executed to completion |
| Blinded judging and human audit | Pinned category rubric, shuffled judge tasks, audit packet generation/preservation tests | Final judgments and two actual human reviews/adjudication are missing |
| Statistical and operational report | Paired confidence interval and retrieval metric protocol tests; operational aggregation and report gates | Implemented; a complete empirical report is missing |
| Runtime and review delivery | `run-memory.sh`, pinned Docker build, successful test script run, rendered localhost UI, `subsystem-review.md` | Runtime and four technical explanations available; final requirement-by-requirement audit remains open |

Earlier runs were explicitly retired when implementation changes or dataset identity
issues invalidated their freezes. Their artifacts are retained for debugging and are
excluded from final benchmark claims. The active full experiment is
`benchmark/runs/review-full-v3/`, with its exact source archive and manifest. It must
finish all conditions, repetitions, ablations, judgments and human reviews before
the project can claim measured improvement or full completion.
