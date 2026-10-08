# General-purpose memory worker: research protocol

**Release:** `aryaniyaps/mem-extractor`. The target is general-purpose conversational memory across everyday and professional domains. The frozen general-purpose corpus contains **2,961 task rows across 16 domains**. Historical coding experiments are documented separately below. Training completion, final evaluation and deployment are established only by their saved receipts; this document does not infer a successful model from corpus size or low training loss.

## Research question and acceptance

Can a small instruction model trained on an 8 GB laptop GPU extract durable, source-grounded facts from varied conversations, consolidate supported observations, and answer with evidence-linked citations—including changed preferences, relationships, plans, routines and professional information?

A successful output must do more than parse as JSON. The operational path includes queued evidence, accepted assertions in PostgreSQL, supported derived observations, Neo4j projection, corrections to prior assertions, retrieval and cited reflection. Schema acceptance, semantic support, reference coverage and live completion are measured separately.

## General-purpose corpus

The frozen `data/research-v3/manifest.json` records:

| Partition | Task rows | Episodes | Extraction | Consolidation | Reflection |
|---|---:|---:|---:|---:|---:|
| Train | 2,095 | 855 | 787 | 530 | 778 |
| Validation | 424 | 160 | 160 | 104 | 160 |
| Test | 442 | 166 | 166 | 110 | 166 |
| Total | 2,961 | 1,181 | 1,113 | 744 | 1,104 |

These episode counts include historical replay. Multiple tasks from one episode are correlated; 2,961 rows are not 2,961 independent conversations. Training has 223 empty-extraction examples and 338 insufficient-evidence reflections, so quantity includes learning when to abstain.

Every partition covers family/friendships, food/dining, travel, education, career/workplace, appointments, household routines, shopping, arts, sports/recreation, community, pets/caregiving, accessibility/communication preferences, wellbeing routines without medical advice, small business, and software/technical projects. Software accounts for 240/2,095 training rows (11.46%); the model target is not restricted to coding.

The general generation run produced 1,280 synthetic episodes. The final label audit retained 823 original passes and 242 repaired-and-reviewed episodes (1,065 total). Rejected labels were repaired against unchanged source evidence and reviewed again; failing cases were excluded. The corpus also includes bounded historical coding replay. The manifest preserves exact counts, selection provenance, prompt versions, split hashes and exclusions.

The teacher is GPT-5.6-sol through authenticated Pi. This is **sequence-level distillation** from generated targets, not teacher-logit/KL distillation. Labels and semantic audits use the same teacher family: they are quality filters, not independent human ground truth.

## Splits, label quality and leakage controls

Scenario-family partitions are allocated before generation. All tasks from an episode retain their family partition; exact prompt overlap is checked across partitions. Historical public traces were split by repository. These controls reduce direct leakage but do not rule out semantic overlap or unknown base-model pretraining contamination.

Each kept row passes the project validators: exact source quotes and valid attribution for extraction, at least two distinct supplied supports for consolidation, and citations limited to supplied evidence for reflection. Semantic review additionally checks entailment, speaker identity, relationship direction, preference exceptions, corrections, rejected suggestions and temporal precision. Null timestamps are permitted when exact time is unknown; invented precision is an error.

[Hindsight is 20/20](https://arxiv.org/abs/2512.12818) and its pinned extraction implementation informed contextualization and attribution. See [PROMPT_DESIGN.md](PROMPT_DESIGN.md) for exact source mapping, prompt revisions and unresolved development-probe failures. This project retains its own slot/evidence schema and validators. Shared prompt hashes prevent historical runs from being mislabeled as using newer prompts.

## Model and actual training lineage

Base: [Qwen/Qwen3-4B-Instruct-2507](https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507), Apache-2.0, revision `cdbee75f17c01a7cc42f958dc650907174af0554`. Non-thinking behavior, compatible export support and feasible quantized adapter training motivated the choice; official model-card scores are not this project's measurements. Qwen3.5 was considered but would introduce another architecture/export path. No global “best base model” claim is made.

Implementation: Transformers + PEFT + bitsandbytes, NF4 double quantization, bf16 compute, rank-16 LoRA, alpha 32, dropout 0.05, attention/MLP projections, answer-only cross entropy, gradient checkpointing and chunked output-head loss. See [PEFT quantization documentation](https://huggingface.co/docs/peft/developer_guides/quantization). Hardware is **RTX 4070 Laptop, Ada, 8 GB nominal VRAM**, not an RTX 5070 Ti.

| Stage | Adapter directory | Training data/exposure | Status evidence |
|---|---|---|---|
| Historical coding stage | `out/local-4b-v2` | 507 rows within 4,096-token cap; 48 excluded from original 555 | Completed adapter and saved run configuration |
| General stage A | `out/local-4b-general-stage-a` | 585 audited rows, all 16 domains, 20% coding; 74 optimizer steps | Completed; 1,415.34 seconds, loss 0.21553, peak allocated VRAM 5.21 GB |
| General stage B | `out/local-4b-general` | 1,510 remaining unique training rows + 128 replay exposures = 1,638 rows | Completed: 205 steps, 4,005.27 seconds including validation, loss 0.17596, validation loss 0.12629, peak allocated VRAM 5.36 GB |

Stages A and B cover 2,095 unique training rows. Their 2,223 combined row exposures include 128 repeated rows and must not be reported as additional unique data. The historical coding stage is inherited through the warm start. Exact learning rates, seeds, epochs, token totals, hashes and exclusions are in each `run-config.json`; completion metrics are in `training-result.json`.

A 7,141-token smoke input exceeded GPU capacity. After reducing the cap, an fp32/bf16 output-head mismatch was fixed; a 4,091-token smoke passed at approximately 6.2 GB peak allocation. Training uses a 4,096-token limit and excludes over-length rows rather than silently truncating targets. Stage A contained 634,454 total tokens and 116,713 answer tokens, mean 1,084.54 and maximum 2,628 tokens per training example. The tested serving context is 12,288 tokens; this does not establish learned long-context accuracy.

## Frozen evaluations and experimental controls

Compare untrained Qwen3-4B-Instruct-2507 and the final adapter with the same Q8 serving quantization, current prompts, constrained decoding and 12,288-token context. The internal held-out sample has **64 distinct episodes**: two extraction, one consolidation and one reflection per domain, selected deterministically before student outputs. Its SHA256 is `0c880b353f675b3b4e3daf520e5911379990034928ea9261f14eb7785ef3f94c`.

A separate **16-window external LoCoMo extraction diagnostic** spans ten conversations and sixteen sessions. Source/derived labels remain local under CC-BY-NC-4.0 and never enter training or the public model package. See [EXTERNAL_LOCOMO.md](EXTERNAL_LOCOMO.md). These short windows are not the official LoCoMo QA benchmark or a long-horizon retrieval evaluation. LongMemEval informed coverage design but was neither trained on nor scored.

Release receipts require completed baseline/student summary and semantic reports for both evaluation sets, with exact denominators and zero grading errors. Report first-attempt acceptance, eventual acceptance, supported-claim rate, reference coverage including failed windows, correction behavior, consolidation and reflection correctness. Same-family teacher judging remains a limitation. Do not claim semantic improvement until the actual matched results support it, or compare timings from differing hardware/concurrent loads.

The paired-source codec binds quote choices to visible event spans and restricts enums and existing-fact identifiers. It converts back to the existing canonical API without relaxing domain validation. Exact text matching does not prove entailment; quoting a rejected proposal can still support an incorrect model claim. Short teacher quotes are widened only to the shortest permitted containing source span, retaining source identity and claim text.

## Final operational and publication gates

`verify_live_memory.py` must pass all required checks using the selected final model: durable assertions, updated current city, superseded historical city, unchanged preference, correct relationship, no invented geography, observation with two supports, Neo4j projection, source-backed recall and cited reflection. `build_release_receipt.py` requires those checks, the correct worker identity, final adapter/GGUF and completed training/evaluation receipts. A saved receipt with a failed or omitted check is not acceptance.

The Hugging Face package is allowlisted: adapter, GGUF, checksums, shared prompts/schema, inference helper, model card and documentation. Credentials, private conversations, teacher caches, raw LoCoMo data and optimizer states are excluded. Public attribution includes inherited Nebius data. The executed notebook records actual experiments and failures; it is not a claim that all original work occurred interactively in Jupyter.

## Historical coding experiments and recovery

The first Qwen3-1.7B adapter learned deterministic synthetic examples well. Naturalistic refinement still accepted only 18/33 extraction windows under the earlier unconstrained decoder. That set became diagnostic development evidence, not a fresh test. Changes to data, model and decoding occurred together, so cross-generation differences cannot be attributed to model size alone.

Historical public data came from [Nebius SWE-rebench OpenHands trajectories](https://huggingface.co/datasets/nebius/SWE-rebench-openhands-trajectories), CC-BY-4.0. Fifty-two sampled repositories yielded 144 windows; 118 relabeled training windows remained before stage-1 token filtering. Provenance records repository, issue, trajectory, ranges and source hashes. This coding exposure is inherited by the final warm-start lineage even without later replay. The limited source slice is Python-heavy.

On 8 October 2026, original blocked extraction job `731699d3-52c1-4d08-af65-fa6b2031a8b6` completed with structured decoding and the 1.7B bridge worker, and projection succeeded. This establishes recovery of that blocker, not final 4B semantic quality. A restart left one job under an obsolete lease; only its verified stale matching lease was expired, preserving evidence and assertions. Final cutovers must account for outstanding jobs and leases.

## Boundaries of the claim

The corpus supports a measured general-purpose English memory experiment across sixteen sampled domains. It does not establish universal reliability, multilingual transfer, production safety or very long-history performance. Synthetic labels and correlated grading can share systematic errors. Independent human review, broader external testing and longer-horizon retention tests remain necessary. Preserve evidence, correction history and deletion controls rather than treating extracted assertions as unquestionable truth.
