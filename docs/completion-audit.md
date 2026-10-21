# Completion audit

Updated on 2026-10-21 after the LoCoMo test run. The authoritative scope is
`project-requirements.json`.

The benchmark was run on LoCoMo instead of LongMemEval-S (the user's choice): ten
conversations, 1,540 questions, the run `benchmark/runs/locomo-final` (commit `fb044f2`).
Every success criterion fixed in `benchmark/config.json` before the run passed. The
results are in `report/latex/5/Chapter5.tex` and `benchmark/Benchmark_Report.ipynb`;
`python3 benchmark/run.py verify --split test --run locomo-final` checks 11,000
answers and 22,000 judgments against the pinned inputs (0 problems). LongMemEval-S and
the two-reviewer human audit were not run.

| Requirement group | Evidence inspected | Current conclusion |
|---|---|---|
| Storage and temporal graph | Migrations 0003–0010, `src/knowledge_store/`, PostgreSQL integration tests, live correction/provenance UI | Implemented; lifecycle, lease recovery, bank serialization and retroactive correction tests pass |
| Sources, assertions and inferences | Exact-source validation, extraction and consolidation repair loops, support checks, invalidation tests, real demo consolidation retry | Implemented; the real invalid-support response was repaired and committed successfully |
| Retrieval and reflection | `src/v2/recall.rs`, graph traversal limits and authoritative validation, lifecycle tests, foreign-citation test, native browser recall/provenance check | Implemented; on LoCoMo the evidence turn is in the packed context for 84.4% of questions and all 99,971 returned quotes are exact |
| Native Pi harness | TypeScript checks, three integration tests, all three actual session JSONLs, `demonstration-report.json`, seven parser unit tests and twelve independent checks | Three sessions and memory injection verified; final-session source-event processing still pending at the time of this audit |
| Dataset and protocol | LoCoMo pinned by SHA-256, frozen manifest and source archive, protocol tests | Complete; faults diagnosed on two dev conversations, eight held out |
| Primary conditions | `locomo-final`: engine, plain retrieval, recent window, full context, no memory on all 1,540 questions | Complete; engine 83.1% vs 80.0% plain retrieval (Jev), all criteria pass |
| Repetitions and ablations | Two repeats and three ablations on a 300-question stratified sample | Complete; reader SD 0.5 pts for the engine; graph and consolidation show no measurable effect on LoCoMo |
| Blinded judging and human audit | Jev and GPT-4o-mini judge every answer without the condition | Judging complete (agreement 96.7%, kappa 0.93); the two-reviewer human audit was not done |
| Statistical and operational report | `report.json`, paired and cluster bootstrap, ingestion and latency, `export_tex.py` tables | Complete |
| Runtime and review delivery | `run-memory.sh`, pinned Docker build, successful test script run, rendered localhost UI, `subsystem-review.md` | Runtime and four technical explanations available; final requirement-by-requirement audit remains open |

Earlier runs (`review-full-v3` and the LongMemEval runs before it) were retired when
implementation changes invalidated their freezes and are excluded from all claims. One fault
was found after the test run, by its property suite: relative dates were stored as the day of
the conversation when the extractor left the event date empty. It was fixed in `43af5bd` and
checked with the property run `properties-v7`; the LoCoMo numbers are for the engine before
that fix.
