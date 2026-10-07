# Technical review: four subsystems

Use these explanations alongside actual code and traces. They describe the implemented
system, not a claim about which student authored which change. The evaluation is still
running; performance and accuracy claims require the final results.

## 1. Temporal storage, concurrency and graph projection

The central problem is preserving evidence while changing beliefs. An episode is an
immutable request, identified by a namespace and idempotency key. The service hashes
the request body: an identical retry returns the existing episode; changed evidence
under the same key returns a conflict. Evidence is retained before inference begins.
Assertions and their exact sources are separate records, so reinforcement can attach
new provenance without inventing a new fact.

Single-valued slots model state; multiple-valued slots model concurrent facts; events
include occurrence time in their identity. A confident explicit correction closes the
old state and creates the replacement. An unexplained conflict marks alternatives
contested. Valid time answers when a belief applies, while recorded time identifies
when the engine learned it. A retroactive correction can make an old belief's interval
empty, but never negative. Current retrieval excludes superseded beliefs; historical
retrieval checks intervals.

The difficult concurrency boundary is between slow model calls and transactional
updates. A job lease authorizes one attempt, and a namespace lease serializes inference
within one bank. Four different banks can progress concurrently. Heartbeats renew both
leases; completion verifies both tokens. A late attempt cannot commit after losing its
lease. Earlier unresolved extraction blocks later extraction, including during retry
backoff, so a faster model response cannot reorder a conversation's corrections.

Neo4j is a projection, not the authoritative store. The PostgreSQL transaction writes
assertions and durable projection jobs together. A projection uses a coherent database
snapshot and monotonically increasing assertion revisions. Neo4j namespace gates and
reset watermarks prevent old writes from resurrecting cleared data. Rebuild replays
the authoritative state. This is an eventual-consistency design: retrieval validates
graph candidates in PostgreSQL and reports projection problems instead of trusting a
possibly stale graph.

Review evidence: `tests/evidence_graph.rs` exercises correction, retroactive validity,
dependency invalidation, lease expiry, concurrent claims and projection/reset behavior.
Inspect a real assertion's source, interval, relation and projection trace in the UI.

## 2. Evidence extraction and supported observations

The worker separates retaining a conversation from interpreting it. Pi runs in an
isolated, tool-free process with a constrained JSON task; Ollama is an alternative
model adapter. Only finalized assistant text is parsed. The model cache keys include
model identity, prompt version and prompt contents, allowing an interrupted job to
reuse inference without treating cached output as a committed database update.

Extraction must pass schema, attribution and relationship validation. Source indices
must exist, each quote must occur verbatim in its cited event, aliases must occur in
supporting evidence, and related IDs must come from the provided existing assertions.
Invalid extraction receives bounded repair attempts with the validation error. Tool
output is evidence of observed execution; an assistant's statement alone does not
prove a command passed.

Consolidation creates observations supported by at least two active assertions in the
same namespace. The engine copies their source provenance and records support edges.
New observations point to existing supports, making the dependency graph acyclic.
Identical observations reuse their identity and attach evidence. Superseding or
retracting a support recursively invalidates descendants and schedules recomputation.
Without this step, summaries can continue repeating a fact that has already changed.

Exact quotations prove attribution, not semantic entailment. A model can still
misinterpret a quote or choose an inconsistent predicate. The independent benchmark
and human review must measure those errors; schema validation alone is insufficient.

Review demonstration: derive a supported observation, correct one support, and show
the old observation becomes stale while its historical provenance remains inspectable.

## 3. Hybrid retrieval, bounded graph traversal and reflection

Recall searches lexical and embedding indexes, optionally plans a temporal range,
then expands a bounded set of graph seeds. The graph explores at most two assertion
hops with explicit per-round and frontier limits. This avoids unbounded variable-path
queries on highly connected entities. PostgreSQL rechecks namespace, assertion state,
validity and support eligibility before a graph candidate can enter the context.

Reciprocal rank fusion combines the candidate lists using the sum of `1/(60 + rank)`.
This combines orderings without assuming that lexical and vector scores share a scale.
UUID ordering resolves ties reproducibly. Whole evidence blocks are packed into an
estimated token budget, with bounded quote lengths and counts. The estimate uses
characters, so it is a packing heuristic rather than an exact model tokenizer count.
Raw source fallback remains attributed when no assertion candidate is available.

Reflection receives only the packed evidence. It must cite retrieved assertion IDs;
foreign citations are rejected, and no evidence produces abstention. Both recall and
reflection persist traces. The observatory exposes component ranks, paths, source
quotes, validity and elapsed stages so a reviewer can explain why a result appeared.

Review comparison: run the same question with graph expansion or observations disabled.
The formal experiment measures both retrieval and answer ablations, rather than using
one attractive graph example as evidence of general improvement.

## 4. Native Pi integration and reproducible evaluation

The Pi extension uses actual session and tool lifecycle events. It recalls before a
new agent task, captures user/assistant exchanges and observed tool outcomes, and
retains commit/branch provenance. Injected memory messages and the extension's own
tools are excluded from reingestion. This prevents remembered context from becoming
new corroborating evidence merely because the harness displayed it.

Retention uses a durable local spool: write and sync locally before sending, then
remove acknowledged batches. Stable identities make reconnect and resume idempotent.
Tests cover offline retry, actual observed exit-code handling, duplicate callbacks
and branch/session resume. The three-session demonstration changes a parser from
milliseconds to microseconds, then asks a fresh session to preserve the remembered
API while fixing malformed inputs. `benchmark/demo/verify.py` checks native memory
injection and independently verifies the final API. This demonstration is separate
from a paired coding benchmark.

The LongMemEval experiment freezes dataset, rubric, configuration, source archive,
model identities and question set. Ingestion constructs requests from a strict
allowlist that excludes questions, answers and evidence labels. Repeated session IDs
retain their scoring identity but use distinct episode keys. All 500 questions use
complete histories in isolated banks. Fresh readers compare raw hybrid, original
enhanced memory and full-history context. A fixed 100-question
sample receives two extra repetitions and answer ablations; retrieval ablations cover
all 500 questions.

Judgments hide condition names and use the pinned upstream category rubrics through
the subscription model. Paired bootstrap intervals compare outcomes on identical
questions. Two actual human reviewers independently label the audit packet, with
adjudication for disagreements. Raw answers, judgments, retrieval and operational
artifacts make the result reviewable. Completion requires every planned condition,
repeat, ablation and human review; a partial report cannot establish improvement.
