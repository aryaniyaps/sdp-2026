# Memory Engine — Review 1

An offline-first, production-shaped Rust tracer bullet for temporal memory. It keeps immutable source events, extracts typed facts with Ollama, versions corrected facts transactionally, and retrieves active memory with PostgreSQL full-text search plus pgvector.

## Start the review

```bash
./scripts/review.sh
```

Open <http://127.0.0.1:8080> (Swagger: <http://127.0.0.1:8080/swagger-ui/>). PostgreSQL is bound only to `127.0.0.1:55432` and the API only to `127.0.0.1:8080`. The launcher checks Docker and Ollama, starts PostgreSQL, ensures both models exist, pre-warms them, applies migrations on API startup, and prints the review URLs.

The deterministic fixture button is deliberately labelled and available only with `DEMO_MODE=true`. It bypasses extraction only; evidence storage, embedding attempts, transactional version resolution, provenance, and retrieval use the same application path. Live extraction never fabricates fallback facts.

## Test

```bash
./scripts/test.sh
```

No host Rust toolchain is required. For database inspection: `docker compose exec postgres psql -U memory -d memory`. See [Review 1 guide](docs/review-1.md) for the demonstration and boundaries.

Token counts use the documented conservative estimate `ceil(Unicode characters / 4)`. Search independently takes 20 lexical and semantic candidates, combines them using reciprocal-rank fusion with `k=60`, then caps results at 20 and packs complete statements within the requested budget.
