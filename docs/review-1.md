# Review 1 demonstration guide

## Three-minute sequence

1. Run `./scripts/review.sh`; open the UI and point out database/extractor/embedder readiness.
2. Leave “Aryan prefers Python for programming” in the input and use **Deterministic fixture** for a fast, repeatable review (or **Live extraction** to exercise Ollama). Search the default question and show Python with lexical, semantic, and fused ranks.
3. Click **Load Rust correction**, ingest it using the same control, and search again. The default context contains active Rust only.
4. In the ledger, show Python as superseded and Rust as active. Expand the visible source quotes, session, validity timestamps, extractor version, and supersedes UUID.
5. Reduce the token budget and search to demonstrate whole-statement bounded packing. Reset to recover the demo at any time.

## Architecture mapping

- `raw_events` and `chunks` are immutable evidence; failures remain recorded with processing state and error.
- `memories` supplies namespace-scoped identity from normalized `subject::predicate`.
- `memory_versions` has one active row per memory, validity intervals, 1,024-dimensional vectors, and generated PostgreSQL search vectors.
- `memory_version_sources` retains many-to-many provenance; `memory_relations` records correction edges.
- Provider traits isolate Ollama extraction and embedding. Extraction uses temperature zero, schema-constrained output, a 120-second timeout, and one schema-repair retry.
- Retrieval runs lexical and semantic queries independently and explicitly reports lexical-only degraded mode when query embedding fails.

## Current boundary

This increment intentionally omits workers, consolidation, retention/expiry, reranking, entity graphs, deletion, authentication, backups, load testing, and LongMemEval/LoCoMo evaluation. Out-of-order corrections return HTTP 409 rather than corrupting temporal intervals.
