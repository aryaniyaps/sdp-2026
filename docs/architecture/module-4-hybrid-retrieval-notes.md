# Module 4 source audit

Verified against the current working tree on 13 October 2026. The drawing continues the illustrative Maya example from Modules 1–3. Event and assertion labels are illustrative; runtime references use UUIDs. Existing diagrams were not regenerated.

## Recall

`src/v2/recall.rs` validates namespace, query, token budget, top_k and optional distance. Lexical search, query embedding, query entity resolution and projection backlog detection begin concurrently. Semantic search follows query embedding. Each candidate channel is bounded to 40 hits. PostgreSQL full-text uses English websearch queries and ts_rank_cd; pgvector uses cosine distance. The local embedding model is `qwen3-embedding-cpu`, with 1,024 dimensions.

Temporal candidates require temporal=true and time bounds. Explicit as_of/from/to filters still apply when temporal=false. The default planner is rules; a model fallback is optional. Current queries select active/contested assertions. Historical as_of can select a superseded assertion whose validity interval contains that instant; an event range alone does not enable superseded assertions. The mixed Maya question does not guarantee that the planner will derive a historical instant. Pi can retrieve historical statements from original source chunks.

Graph seeds combine up to five lexical IDs, five semantic IDs and ten resolved entity matches, deduplicated and truncated to twenty. Expansion uses two bounded rounds over MENTIONS and SUPPORTS. PostgreSQL rechecks graph candidates for namespace, status and temporal eligibility, followed by the configured distance check. The drawing groups all seed inputs on one connector; semantic results are not their sole source.

Source-text lexical/vector retrieval runs when include_raw is requested or fallback conditions apply, including unprocessed extraction or absent assertion candidates. Reciprocal rank fusion sums 1/(60+rank) over channel ranks, deduplicates IDs, and resolves ties by UUID. Supporting data is loaded in batches. Packing applies top_k and estimated token budget, with up to three source quotes per hit, each clipped to 1,200 characters. Token estimates use characters divided by four.

The recall response includes cited context, results, ranking, a trace ID, degraded reasons and temporal plan. Embedding or graph failures can leave partial channels available; ordinary database failures can still fail the request. The diagram's shaded recall area groups candidate retrieval and ranking; evidence packing and context immediately below it are also part of recall_engine.

## Pi and optional reflection

The Pi before_agent_start integration recalls the current prompt using the active namespace, temporal=false, include_raw=true, max_distance=0.45, top_k=10, max_tokens=2,048 and a 15-second timeout. Nonempty context is injected only if the namespace still matches, with an evidence-not-instructions notice. Pi then uses it with its conversation. Automatic Pi recall does not call reflection.

`src/v2/reflect.rs` implements a separate POST /api/v2/reflect endpoint which calls the same recall engine. With no hits it returns insufficient evidence and empty citations. Otherwise it calls the configured local `mem-extractor:general-v1` model through cached_generate using reflect-v2.1. Validation checks the answer and insufficient_evidence fields, ensures citation UUIDs belong to packed recall results, and requires citations when sufficient evidence is claimed. Invalid output returns an error without a repair loop. Reflection returns answer, citations, insufficient_evidence and recall details. The dashed connector represents reuse of recall output within that explicit endpoint.

## Validation

Checked source contracts and diagram connector references, then rendered the PDF and inspected its full-page layout. Exported editable draw.io, PNG, SVG and PDF. No application code or runtime tests changed.
