# General-purpose extraction prompt revision

The product target is general-purpose memory. The completed coding-focused corpus and 4B stage-1 adapter are initial experiments, not evidence of general-purpose capability. The automatic coding-only stage-2 launcher was stopped before training began. Existing artifacts remain intact.

## Primary sources

- Latimer et al., *Hindsight is 20/20*, arXiv:2512.12818v1, §4.1.2 and Appendix A.1, printed pp. 23–24: https://arxiv.org/pdf/2512.12818v1#page=23
- Current implementation, pinned commit `1152717735237c26986b877789704b89bac89681`, `_BASE_FACT_EXTRACTION_PROMPT`, `_CONCISE_GUIDELINES`, `_DEFAULT_LANGUAGE_RULE`: https://github.com/vectorize-io/hindsight/blob/1152717735237c26986b877789704b89bac89681/hindsight-api-slim/hindsight_api/engine/retain/fact_extraction.py

The paper motivates contextual facts with participants, timing, location and stated motivation, and explicit coreference resolution. Its appendix requests exhaustive detail. The current implementation also offers selective concise extraction and treats missing dimensions explicitly. We adapt these principles in original wording to this project's schema; we do not copy Hindsight's prompt or claim to reproduce its system or benchmark results.

## Adaptation decisions

| Concern | Our implementation |
|---|---|
| General scope | Preferences, relationships, routines, plans, experiences, learning, work and technical projects |
| Narrative context | Self-contained statements retain relevant reasons/exceptions; independently mutable attributes remain separate slots |
| Attribution | Reports, beliefs, suggestions, rejected plans and completed actions remain distinct |
| Entity references | Resolve names/relationships only with clear evidence; cite both identity and value events when necessary |
| Temporal precision | Anchor relative dates to source time; date-only statements do not acquire fabricated clock times |
| Missing dimensions | Omit unknown details; never require a fabricated who/where/why |
| Evidence | Exact quotes and source indices remain mandatory; matching text alone does not prove entailment |
| Changes | Stable subject/predicate, explicit correction, no invented replacement after retraction |
| Example contamination | Remove the concrete payments/pnpm output example and the full default-valued example; describe the required fields and per-claim decisions |
| Capacity | Compact instructions retain room for prior facts and repairs on the 4,096-token context tests |

Hindsight has interval dates and a different fact/network schema. This engine currently has `event_at` and `valid_from`, not an occurrence interval. We therefore retain coarse dates in text and avoid encoding a made-up exact instant. No schema fields were silently added. The system's null `valid_from` fallback remains mention time, not proof of the real-world start date.

## Verification and limits

The first 7,016-byte draft failed four existing context-budget tests because no prior facts fit. It was shortened to 5,361 bytes; all 17 Rust prompt tests passed without weakening the tests or increasing the context budget. Python/Rust parity fixtures are regenerated from shared templates. The extraction contract and validators remain unchanged.

`evaluate_prompt_revision.py` compares the original coding prompt and revised general prompt on the same eight manually specified development probes: preference exceptions, relationship coreference, changed plans, date precision, rejected suggestions, injection, empty chatter and attributed beliefs. Receipts retain model identity, prompt/system hashes, raw outputs, validation attempts and semantic review criteria. These probes are diagnostic, not a fresh generalization benchmark. Results must be reviewed before asserting prompt-quality improvement.

The broader corpus must be regenerated/rendered with the revised shared prompts. Historical dataset hashes and old run results must not be rewritten to imply they used this revision. Final general-purpose training and domain-stratified evaluation remain outstanding.

## Measured prompt iterations

The original coding prompt retained only a simplified train preference among the personal probes, dropping its exception. The first general revision restored useful personal content but accepted an injected yacht assertion. Revision 2 added explicit treatment of memory-control directives and rejected that injection. Revision 3 moved key rules into the system message but still borrowed a season from a rejected suggestion. Revision 4 removed the full default-valued output example and added explicit per-claim checks: the destination update now has correction=true, the rejected suggestion is skipped, and the preference exception is preserved.

All eight revision-4 responses passed structural validation. Semantic inspection still found unresolved relative dates, fabricated event timestamps and incomplete cross-event identity support; one relationship slot disagreed with its narrative statement. These results are diagnostic, not eight successful semantic tests. Full receipts and inspection notes are in `data/research-v3/prompt-probes/`. The current prompt is 5,649 bytes; 59 Rust unit tests, 14 Ollama contract tests and 7 focused Python tests passed. No test expectations were weakened to accommodate semantic failures.

A new 320-batch general-purpose corpus generation run has started across 16 domains, with scenario-family splits allocated before generation. Targets still require semantic audit. Generation began with the revision-3 contract; preserve that provenance and render accepted training rows with the final prompt separately, retaining both hashes. No new model is promoted merely because its prompts or corpus changed.
