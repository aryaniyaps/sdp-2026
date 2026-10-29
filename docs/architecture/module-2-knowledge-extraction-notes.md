# Module 2: source audit

Verified against the current working tree on 13 October 2026, including the uncommitted correction/entity review implementation. Module 1 and the overall diagram are unchanged. This is a logical component diagram; generation nodes share one configured model client rather than representing separate model deployments.

## Extraction

- `src/worker/extraction.rs::extract`: loads ordered episode evidence. It snapshots up to 80 active/contested non-observation assertions in the same namespace, storing that snapshot in the leased job payload for reuse on retries.
- `src/worker/window.rs`: compacts prompt-only events, preserves full evidence for validation, shortens content above 1,600 characters to a 1,100-character head and 360-character tail plus omission marker, and groups windows under an estimated 2,800-token event budget. These are event-budget estimates, not the entire prompt size.
- `src/worker/prompts.rs`: builds the extraction prompt and fits the existing-assertion snapshot to the model context budget. Snapshot entries may be dropped to fit. Source indices start at zero within each window.
- `src/model.rs` and `src/model/structured.rs`: cached generation uses model identity, prompt version and prompt content. Structured decoding schemas and output canonicalization precede domain validation; wire-format paired sources become canonical source_indices/quotes. Generation uses Ollama `/api/generate`, think:false, nonstreaming output and temperature 0, with context preflight and response checks.
- `src/worker/extraction.rs::extract_window`: parses Extraction, validates each claim and checks that related assertion IDs were supplied in the snapshot. There is one initial attempt and at most two validation repair attempts; exhausted invalid chains are evicted from cache. Failures return to the durable job retry mechanism.
- `src/knowledge.rs::validate_claim`: requires nonempty fields, finite confidence in [0,1], valid entity/kind/relation enums, valid paired source indices and exact nonempty quote substrings in the referenced full events. This verifies quote provenance, not semantic truth.
- `src/worker/review.rs`: separately prompts the same model to review each claim. A correction can be vetoed, never promoted from false to true. Change evidence must be a nonempty substring of a validated claim quote. Optional added entity names must occur in the quotes and cannot introduce aliases; unsupported additions are omitted. Subject identity and existing mentions remain. It revalidates claims and permits one repair attempt.
- Extraction assembly drops claims with credential-bearing statements, remaps window-local indices into episode indices, retries absent source embeddings, and embeds each claim statement. Claim embedding failures are represented by absent vectors, not fabricated vectors.
- `Store::apply_extraction` receives the reviewed Extraction, optional vectors and model identity. Entity resolution, assertion persistence, correction/supersession decisions and downstream job creation belong to Module 3.

## Consolidation

- `src/worker/consolidation.rs`: runs as a separate `consolidate` job. It queries at most 100 active non-observation assertions for subjects affected by the episode or changed assertion. Fewer than two input facts results in a no-op.
- Uses the same model through cached generation, version `consolidate-v2.3`. The model proposes ObservationInput records with subject, statement, predicate/value, confidence, explanation and supporting assertion IDs.
- Validation requires at least two distinct support IDs from the supplied facts, finite confidence in [0,1], nonempty statement/predicate/value and at least one supporting fact belonging to the observation subject. There are at most two validation repairs after the initial attempt.
- Observation statements are embedded using the same source/claim embedder. `Store::apply_consolidation` rechecks current support status in its transaction and persists observation assertions, source/entity links and observation-to-support relations. Those persistence details belong to Module 3.
- Extraction and consolidation use the shared Tokio inference worker pool and serialize per namespace. Each job acquires its own lease; the consolidation job does not reuse the extraction job's token.

## Configured models and limits

The local `.env` selects `mem-extractor:general-v1`, `qwen3-embedding-cpu`, 12,288 context tokens, 3,072 output tokens, and two inference workers. These are configured values, not measurements of running requests. The embedding alias is the previously verified CPU-only Qwen3-Embedding 0.6B Q8_0 model; the provider requests and validates 1,024 dimensions. `EXTRACT_VERSION` is `extract-v3.0`, review is `claim-review-v1`, and consolidation is `consolidate-v2.3`.

## Example labels and verification

P2/C3/C4/E3/E4 abbreviate Maya's second-conversation evidence. The extraction arrow shows an illustrative claim, not a recorded model response. Maya's earlier residence is available from the existing snapshot. A1/A2/A3 in the consolidation lane are illustrative work-assertion IDs, not event IDs. The expected work observation is optional, and support checks do not guarantee the model will propose it.

Checked source contracts, connector references and a rendered PDF. No runtime code or tests changed. `build_module2.py` only generates this module diagram; the editable drawio file is the delivered source.

## Simplified visual revision

The user-edited drawio file is authoritative. The original builder predates those edits and must not be rerun over it. The revised view removes explanatory paragraphs and uses component symbols for processing, cylinders for stored records, diamonds for validation, and dashed repair loops. Exact model names, vector size, Maya examples and Module 3 entry points remain visible. Detailed source limits and validation rules are retained in these notes rather than repeated on the diagram.
