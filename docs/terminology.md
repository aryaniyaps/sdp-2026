# Terminology

The project used "evidence" for the raw input it stores and "fact" for what a model extracts from that input. Both names point the wrong way. A fact is something established, the base other things are derived from, but our extracted units are derived, can be wrong, and can be contested, superseded or retracted. Evidence is a role: something is evidence *for* a conclusion. A stored message is not evidence until something cites it. The names below follow the knowledge-representation usage of claims and their sources (Wikidata statements and references, nanopublication assertions and provenance, W3C PROV derivations) rather than the "fact" naming common in agent-memory products.

| Concept | Name | Was | Where it lives |
|---|---|---|---|
| Immutable input: a message or tool result, as received | **source event** | evidence, evidence event | `raw_events`, `SourceEvent` |
| A retained request grouping source events | **episode** | (unchanged) | `episodes` |
| A searchable slice of a source event | **source chunk** | evidence chunk | `chunks` |
| The exact passage an assertion cites | **source quote** | evidence source, evidence quote | `assertion_sources`, `SourceQuote` |
| A typed, temporal statement extracted from sources by a model | **assertion** | fact | `assertions` |
| The assertion kind for a stable property of something | **attribute** | `fact` | `assertions.kind = 'attribute'` |
| A statement derived from two or more assertions | **inference** | observation | `assertions.kind = 'inference'` |
| The job that derives inferences | **consolidation** | (unchanged) | `memory_jobs.kind = 'consolidate'` |

## Rules

- "Evidence" is used only for something cited in support of a conclusion: the passages a reflection answer cites (`insufficient_evidence`), and LongMemEval's evidence sessions, which are that benchmark's own term for the sessions containing an answer.
- "Fact" is used only in its ordinary sense of something established, as in "a contested assertion is not an established fact". It never names a stored record.
- Generative Agents uses "observation" for a raw perception and "reflection" for a derived one, so "observation" also pointed the wrong way for a derived unit. "Reflection" is not used because `/reflect` already names question answering.

## The model contract is unchanged

The extraction, consolidation and reflection prompts (`src/worker/extract_prompt.txt`, `src/v2/reflect_prompt.txt` and the consolidation prompt), the JSON the model returns, and the `slm-distill/` training data still say `fact`, `observation`, "Existing facts" and "Evidence". The distilled model was trained on that exact text, and `src/model/structured.rs` parses it. Changing it would move the model off its training distribution. The service translates at that boundary instead (`crate::knowledge::kind_to_model` and `kind_from_model`). The next retraining can adopt the new names and remove the translation. The review prompt (`src/worker/review_prompt.txt`) was written for the same model and keeps its wording, so all four prompts change together.

Migration files keep their original names (`0003_evidence_graph.sql`, `0007_evidence_quotes.sql`) because `sqlx` checksums applied migrations. `0013_terminology.sql` performs the rename on existing data.

## Wire changes

| Before | After | Compatibility |
|---|---|---|
| `kind: "fact"` in responses | `kind: "attribute"` | No request takes a kind; the model's `fact` is translated on the way in |
| `kind: "observation"` in responses | `kind: "inference"` | Same, for the consolidation model's output |
| Recall request `observations` | `inferences` | `observations` still accepted |
| `stale_observations` (status, retract) | `stale_inferences` | none |
| Consolidation job result `observation_ids` | `inference_ids` | none |
| Extraction job result `existing_facts_dropped` | `existing_assertions_dropped` | none |
| Raw recall hit `status: "evidence"` | `status: "raw"` | none |
| OpenAPI schemas `EvidenceEvent`, `EvidenceSource`, `ObservationInput` | `SourceEvent`, `SourceQuote`, `InferenceInput` | JSON shape unchanged |
| Recall context line `Evidence (session, role, time): quote` | `Source (session, role, time): quote` | Reflection still renders the model's original labels |
