# External conversational extraction diagnostic

Source: Maharana et al., *Evaluating Very Long-Term Conversational Memory of LLM Agents*, ACL 2024 ([paper](https://aclanthology.org/2024.acl-long.747/), [official repository](https://github.com/snap-research/locomo)). The released benchmark contains ten long conversations and separate QA/event annotations. These are externally produced, LLM-generated conversations with human evaluation/annotation, not new human chat logs collected for this project. This experiment does not run the official QA benchmark.

Pinned source revision: `3eb6f2c585f5e1699204e3c3bdf7adc5c28cb376`, file `data/locomo10.json`. The [upstream license](https://github.com/snap-research/locomo/blob/3eb6f2c585f5e1699204e3c3bdf7adc5c28cb376/LICENSE.txt) is **CC-BY-NC-4.0**. Source data and derived labels are retained locally for this academic evaluation; neither is included in the Hugging Face package or any training curriculum. The public model remains distinct from this evaluation-only data.

## Local protocol

`prepare_external_locomo.py` freezes sixteen six-turn windows from distinct sessions, spread across the ten conversations. Hash ordering and input-length eligibility determine selection before any student output is observed. Frozen receipts preserve source revision/hash, conversation/session, dialog IDs, turn ranges and window hash. This is a small external distribution diagnostic, not a statistically representative benchmark sample.

Both named participants are represented as user-role events with explicit speaker prefixes. Image URLs/captions and upstream QA answers are excluded. Upstream session times lack timezones; an API-required UTC suffix is an adapter convention, not evidence of actual UTC occurrence. Labels must not infer precise event time from that convention.

A Pi teacher labels at most three durable claims under the current extraction contract. Each label passes structural validation and separate same-family semantic review; repair attempts remain recorded. This reduces obvious label errors but provides no independent human ground truth. Empty extraction is allowed. None of these rows may enter model training or prompt tuning.

Evaluate the baseline and final student on the identical frozen file. Report validity, evidence support and coverage with denominators, alongside the same-family grading limitation. Label results **external LoCoMo-window extraction diagnostic**, never “LoCoMo QA accuracy” or “official LoCoMo score.”

## Preparation receipt

The frozen extraction file contains **16 windows, 40 reference claims, 10 conversations and 16 distinct sessions**. All rows passed the production-compatible extraction validator and the separate automated semantic review. Label/repair/audit preparation used 58 teacher calls; all attempts remain in local receipts. Frozen canonical file SHA256: `a7de115cfa8da3a164aad8c21359111cb87cca1dd2f058cf8a866c68e274dcf0`. This receipt records label preparation, not student performance.
