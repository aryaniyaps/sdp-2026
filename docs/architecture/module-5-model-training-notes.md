# Module 5 source audit

Verified against the current working tree and saved experiment receipts on 13 October 2026. This is an architecture diagram, not a new training or evaluation run. Maya is an illustrative task record continuing the other diagrams, not a claim that her exact messages were in the frozen corpus. Live conversations are not automatically ingested for training.

## Corpus and labels

`slm-distill/expand_general_corpus.py` generates synthetic episodes with scenario-family partitions assigned before generation. `pi_teacher.py`, `aux_tasks.py` and `prompt.py` provide teacher calls and task prompts. The recorded teacher is GPT-5.6-sol through authenticated Pi. Shared extraction, consolidation, reflection and system templates connect the offline pipeline to runtime contracts. Saved prompt hashes identify the versions used by each experiment; current files need not be identical to every historical run.

`audit_general_corpus.py`, `repair_general_corpus.py` and `assemble_general_corpus.py` implement audit, repair and assembly. Kept labels must satisfy task-specific constraints: exact quotes and attribution for extraction, at least two distinct supplied supports for consolidation, and citations from supplied evidence for reflection. Semantic audits check entailment and temporal precision. Repair preserves source evidence and is reviewed again; failing examples are excluded. The diagram combines structural and semantic checks in one decision.

The frozen corpus has 2,095 training, 424 validation and 442 test task rows across sixteen domains. Multiple tasks from an episode are correlated. Historical coding replay is included; scenario-family and exact-prompt disjointness checks reduce direct split leakage. The separate external LoCoMo diagnostic is not part of this training corpus. This is sequence-level distillation from teacher targets, not teacher-logit distillation.

## Training and export

`train.py::encode` renders the chat prompt, masks prompt labels with -100 and supervises target tokens including the end marker. Over-length examples are excluded and logged rather than truncated. The run uses a 4,096-token cap.

NF4 QLoRA loads Qwen3-4B-Instruct-2507 in four bits with bf16 computation. Rank-16 adapters cover attention and MLP projections. The base stays frozen; an existing adapter can be resumed. AnswerOnlyTrainer applies cross-entropy to answer positions, using chunked output-head computation and checkpointing to reduce memory. Recorded runs use batch size one and gradient accumulation eight on an 8 GB RTX 4070 laptop GPU.

The saved lineage is historical coding training (507 included rows), general stage A (585 rows), then general stage B (1,510 new rows plus 128 replay exposures). Stages A/B cover 2,095 unique general rows. The warm-start arrow represents inherited adapter weights, not training from an untrained adapter. `export.py` merges base and adapter, converts to GGUF and quantizes Q8_0. The export retains the non-thinking chat template and checksums.

## Evaluation, acceptance and release

`evaluate_research.py` and `grade_research.py` compare base and student under matched Q8 quantization, prompts, decoding and 12,288-token serving context. The internal selection contains 64 distinct episodes; the external diagnostic has sixteen held-out LoCoMo windows. This is not the official LoCoMo QA benchmark. Evaluation includes structural acceptance, support, coverage and task correctness. Frozen matched results predate the later production reflection citation-enum repair; live acceptance separately evaluates that repair.

Saved internal results in `reports/RESEARCH_PROTOCOL.md` and `data/research-v3/release-receipt.json`: consolidation 1/16 to 13/16, reflection 10/16 to 15/16, extraction reference coverage 50/55 to 41/55. This is a multitask tradeoff with extraction regression. The labeler and grader use the same teacher family; scores are not independent human accuracy.

`verify_live_memory.py` checks retained evidence through assertions, corrections, observations, graph projection, recall and reflection in a fresh namespace. `build_release_receipt.py` checks required artifacts and operational results, including worker identity. The release decision means the configured checks passed; it does not require every semantic score to beat the base. `stage_release.py` allowlists package artifacts; publication is a separate controlled step. The diagram compresses staging, verification and publication into the successful release connector.

`scripts/fetch-slm.sh` obtains the model package, checks the GGUF checksum and imports it into Ollama. The configured model name is `mem-extractor:general-v1`. Module 2 uses it for extraction/consolidation and Module 4 for optional reflection. The query/document embedding model is separate and is not trained by this pipeline.

## Diagram checks

Verified unique XML IDs and connector references, exported a one-page PDF and visually inspected the final render for clipping and overlaps. Exported PNG and SVG from the same editable drawing. Existing diagrams, runtime source, datasets and model weights were not modified.

## Presentation revision

The user removed the historical coding adapter from the drawing and simplified its content. Subsequent edits operate directly on the user-edited draw.io file; the original builder must not be rerun over it. The diagram now uses document symbols for examples, prompt contracts, weight artifacts and reports; a cylinder for the frozen dataset; component symbols for processing/services; and diamonds for decisions. The explicit frozen base-model document supplies both QLoRA training and merge/export. Historical lineage, detailed audit criteria and measured tradeoffs remain in these source notes rather than the condensed diagram. Repair is bounded by the implementation's exclusion policy; the compact feedback loop does not imply unlimited retries.
