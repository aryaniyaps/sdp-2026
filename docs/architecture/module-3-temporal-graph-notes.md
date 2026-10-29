# Module 3 source audit

Verified against the current working tree on 13 October 2026. Module 1, Module 2 and the overall diagram were not modified. The drawing uses logical components, a transaction boundary, decision diamonds, PostgreSQL storage, a durable job queue and an example of the actual projected graph.

## Persistence and entity resolution

`src/knowledge_store/assertions.rs::apply_extraction` reloads episode evidence, checks vector count and validates claims before opening its transaction. `lock_namespace` takes a PostgreSQL transaction advisory lock. The apply path checks that both job and namespace leases still belong to the caller and have not expired. The diagram groups these under the apply boundary; validation begins before the database transaction.

`src/knowledge_store/mod.rs::entity` resolves by namespace, type and normalized name, then by one unambiguous stored alias; otherwise it creates a row. There is no similarity-based merge. Aliases are stored only when cited evidence contains them. The assertion slot is subject entity UUID plus normalized predicate.

## Assertion decisions

An existing active/contested assertion with the same normalized value and cardinality is reinforced, preserving its ID and adding provenance/entity links. Event cardinality additionally matches event_at. A new unmatched value with no conflicting single-valued slot creates an active assertion. Non-single values do not automatically supersede other values.

A differing single-valued claim supersedes prior active/contested assertions only when correction=true and confidence >= 0.8; otherwise prior and new assertions become contested. Supersession closes the old interval with `valid_to = max(old.valid_from, new.valid_from)`. New valid_from uses a supplied effective instant or the first cited event timestamp. recorded_at remains separate. The Maya dates assume the previously illustrated 1 October and 10 October facts; they do not establish the true start of her earlier residence.

`assertion_sources` links assertion to chunk and exact quote. `assertion_entities` links the subject and other extracted entities. `assertion_edges` stores supersedes/contradicts and optional model-related extends/contradicts/causes. Explicit related contradictions can also update linked assertion statuses; the drawing shows the primary same-slot path, while this additional relation path is handled in the same apply transaction. Reinforcement advances the projection revision too.

## Observations and invalidation

`apply_consolidation` independently acquires the namespace lock and checks its own live leases. It locks and rechecks at least two distinct active support assertions in the namespace and requires the subject to belong to a support. Identical active observations with the same support set are reused and their provenance refreshed. Otherwise older observations in the slot are marked stale and a new observation assertion is inserted. Observation valid_from is the maximum support valid_from. Source and entity links are copied; edges point observation → supporting assertion with relation supports.

`invalidate` recursively follows supports/derives dependencies and marks affected active/contested observations stale, queuing projection for them. It runs within the mutation transaction, not as an asynchronous inference step. Retraction also invokes it and queues consolidation for the changed assertion. The diagram's retraction label is a trigger, not a second leased extraction entry point.

Extraction commits the assertion mutations, processed-event state, projection/consolidation jobs and extraction-job success together, then releases its namespace lease. Consolidation commits observations, projection jobs and consolidation-job success together. Job payloads carry IDs; projection reads fresh authoritative state later.

## Graph projection

`src/worker/projection.rs::project` reads a single MVCC-consistent PostgreSQL snapshot containing assertion properties, revision, entity mentions and directed assertion relations. Embeddings, search vectors and source quotes are excluded. `src/graph.rs::project_assertion` uses Cypher over the Neo4j HTTP API. Namespace revision fences and per-assertion revisions reject stale projection writes. Existing outgoing relations are refreshed.

Actual node labels: Assertion and KnowledgeEntity. Actual edges: Assertion → MENTIONS → KnowledgeEntity; all assertion-to-assertion kinds use physical SUPPORTS edges with a relation property. Thus Maya's replacement is R2 → SUPPORTS {relation: 'supersedes'} → R1, not a physical SUPERSEDES relationship. PostgreSQL remains authoritative; Module 4 validates graph candidates against current PostgreSQL status/temporal filters.

## Example and validation

R1/R2 are Maya's old/new residence assertions. W1/W2/W3 are illustrative employer, role and remote-work assertions; O1 is their optional supported observation. C3/E3 is the source chunk/event for the move. Names and IDs are illustrative, not a fresh inference result. Module 3 makes no model calls; it consumes the claims and optional vectors provided by Module 2.

Verified source contracts, XML identifiers/connector references, and rendered PDF layout. No runtime code or tests changed. The builder writes only the Module 3 diagram.
