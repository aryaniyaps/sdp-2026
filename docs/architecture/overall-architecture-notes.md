# Overall architecture: source audit

Audited against the current working tree on 13 October 2026, including the local `worker/review.rs` changes. This is a logical module view of one Rust service, not separate deployed microservices. The edited `.drawio` is authoritative; do not regenerate it using the original `build_overall.py`, which predates the user's layout and wording edits.

## Verified lifecycle

1. Pi's extension retains user and assistant messages and eligible tool outcomes. Retain creates a session (or reuses it), an episode, raw events and one full-message chunk per event. `episode_sources` records chunk order. It queues an `extract` job whose initial payload is only `episode_id`; the conversation is loaded from PostgreSQL, not carried inline in the job. No entities or assertions are created by retain.
2. The extraction worker loads evidence and a stored snapshot of existing assertions, windows the events, calls the model, validates claims and source quotes, runs correction/entity review, and embeds claims. `ClaimInput` includes the subject entity description, other entity descriptions, predicate, value, statement, cardinality, kind, confidence, correction, source indices/quotes, timing and related existing assertion IDs. This model output is not yet the authoritative stored assertion.
3. `apply_extraction` checks leases and commits a transaction. It creates/reuses entities, identifies an assertion slot by subject entity ID plus normalized predicate, creates or reinforces an assertion, writes provenance/mention/relation links, queues projection and consolidation jobs, and completes the extraction job.
4. Consolidation reads active non-observation assertions. It can create an assertion with kind `observation` when at least two distinct active assertions support it. It copies source and entity links from supports; a support change can invalidate dependent observations. It does not need to create an observation in the diagram's single-slot correction example.
5. Projection reads a coherent assertion snapshot from PostgreSQL, including linked entities and assertion relations. It projects Assertion and KnowledgeEntity nodes into Neo4j. Quotes remain in PostgreSQL.
6. Recall searches eligible assertions (and raw-source paths where applicable), expands the graph, rechecks PostgreSQL state, fuses ranks, and packs bounded evidence blocks. Blocks contain ID, statement, kind/status, dates, and up to three source quotes with session, role and time. Pi's automatic before-turn recall uses a relevance distance threshold and disables model temporal planning. Nonempty context becomes an `sdp-memory-context` message described as evidence, not instructions. This path does not call reflection or produce a standalone answer for the human.

## Record distinctions and relationships

- `entities`: namespace, entity type, display name and normalized name. Resolution tries exact namespace/type/normalized-name matching, then a unique stored alias match, then creates a row. No similarity-based entity merging is implied.
- `assertions`: subject_id, predicate, text value, statement, cardinality, kind, status, confidence, validity, provenance metadata and embedding. `fact`, `preference`, `profile`, `goal`, `episode`, `procedure`, `other` and `observation` are assertion kinds, not successive processing stages or separate fact tables.
- `assertion_sources`: assertion ID -> chunk ID plus exact source quote.
- `assertion_entities`: links an assertion to its subject and other extracted entities. A text value such as Bengaluru is not automatically a separate entity; it must be supplied as a grounded entity description.
- `assertion_edges`: directed assertion-to-assertion links. A replacement assertion points to the older one with `supersedes`. An observation points to its supporting assertions with `supports`.
- Neo4j: Assertion -> MENTIONS -> KnowledgeEntity. The current projector represents all assertion-to-assertion kinds with the physical relationship type SUPPORTS and a `relation` property. Thus a correction is `SUPPORTS {relation: 'supersedes'}`, not a physical SUPERSEDES edge in this implementation.

## Example qualifications

The example uses Maya across 1, 10 and 13 October 2026. E1–E5 are readable labels for generated event identifiers. E1 supplies the original residence and work facts; E3 is the user's original move evidence; E4 repeats it. The diagram preserves the provided conversation text and expected answer, with the third query in M4.

The output is illustrative, not a live inference receipt. Matching Maya across conversations assumes compatible namespace, entity type and name or a unique stored alias. Supersession requires matching subject/predicate, single cardinality and a reviewed correction with confidence >= 0.8. Otherwise competing values can become contested. Repeated work claims reinforce existing assertions. Optional work consolidation requires at least two distinct active supports.

Chennai is retained as superseded with its earlier validity interval; Bengaluru becomes active from 10 October. Source time is the default when no supported effective time is supplied. This is not proof of the real-world start of the earlier residence.

The expected answer includes current and past residence plus work evidence. Pi automatic recall uses temporal:false and does not guarantee retrieval of superseded assertions. Historical assertion lookup must use the appropriate temporal query; raw-source fallback can separately supply history. The diagram explicitly labels the complete answer as intended behavior, not a demonstrated automatic recall result. Pi consumes cited evidence context; reflection is a separate model-backed path.

M5 uses curated training data. Retaining these live conversations does not automatically enroll them in training.

## Source anchors

- `src/knowledge_store/evidence.rs`: `retain`, `episode_evidence`
- `src/knowledge.rs`: `ClaimInput`, `AssertionView`, `validate_claim`
- `src/worker/extraction.rs`: `extract`, `extract_window`
- `src/worker/review.rs`: `review`, `apply_review`
- `src/knowledge_store/mod.rs`: `entity`, `project`, `invalidate`
- `src/knowledge_store/assertions.rs`: `apply_extraction`, `apply_consolidation`
- `src/worker/consolidation.rs`: `consolidate`
- `src/worker/projection.rs`: `project`
- `src/graph.rs`: `GraphStore::project_assertion`
- `src/v2/recall.rs`: `FILTER`, evidence-block construction in `recall_engine`
- `integrations/pi/extension.ts`: `before_agent_start`, retention event loop
- `migrations/0003_evidence_graph.sql`: entities, assertions, source/mention/relationship tables

No runtime code or tests were changed. Verification consisted of source tracing, diagram reference checks and visual export inspection; no live inference claim is made.
