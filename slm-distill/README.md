# Memory model fine-tuning

Training and evaluation scripts for `mem-extractor`, the project's conversation memory model. It extracts facts with source quotes, combines facts into observations, and answers questions with citations.

The final model is **Qwen3-4B-Instruct-2507**, trained with NF4 QLoRA on an 8 GB RTX 4070 laptop GPU. The training cap is 4,096 tokens. The saved model is at [aryaniyaps/mem-extractor](https://huggingface.co/aryaniyaps/mem-extractor/tree/77851da91c15fd4e20018a1b0ace13d29e7eb635).

## Results

The dataset has 2,961 task rows: 2,095 train, 424 validation and 442 test, across sixteen domains. These come from 1,181 episode IDs including replay; multiple tasks from an episode are not independent conversations.

On the internal matched test, consolidation improved from 1/16 to 13/16 and reflection from 10/16 to 15/16. Extraction coverage fell from 50/55 to 41/55 reference facts. Relative dates and invented timestamp precision are still problems. Labels and grading use the same teacher family, so the scores are not independent human accuracy.

- [Notebook](Finetuning_Iteration_Report.ipynb) / [HTML copy](reports/Finetuning_Iteration_Report.html): experiments, plots, failures and saved results.
- [Research protocol](reports/RESEARCH_PROTOCOL.md): counts, splits, training stages and evaluation details.
- [Prompt notes](reports/PROMPT_DESIGN.md): prompt changes and development checks.
- [External evaluation](reports/EXTERNAL_LOCOMO.md): sixteen held-out LoCoMo windows, not the official QA benchmark.
- [App setup](../README.md#running-locally-including-offline): databases, Ollama and the demo.

## Setup

From the repository root, with `uv`, Python 3.12 and an NVIDIA driver:

```bash
bash slm-distill/setup.sh
uv pip install --python slm-distill/.venv/bin/python -r slm-distill/requirements-notebook.txt
env -u PYTHONPATH slm-distill/.venv/bin/python -m ipykernel install --user --name sdp-distill --display-name 'SDP distillation (.venv)'
env -u PYTHONPATH slm-distill/.venv/bin/jupyter lab slm-distill/Finetuning_Iteration_Report.ipynb
```

`PYTHONPATH` is cleared because externally injected packages can interfere with the environment. To rebuild the notebook and refresh its outputs:

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/build_notebook.py --execute
```

Run All reads local results and redraws plots. It does not train, call the teacher, change databases or publish anything. Large datasets, checkpoints and teacher caches are ignored by Git. Saved outputs are included for reading; rebuilding without local data marks those results unavailable.

## Files

| Step | Scripts |
|---|---|
| Generate conversations | `expand_general_corpus.py` |
| Review and repair labels | `audit_general_corpus.py`, `repair_general_corpus.py` |
| Freeze splits and measure lengths | `assemble_general_corpus.py`, `general_corpus_token_stats.py` |
| Train | `train.py` |
| Evaluate and grade | `evaluate_research.py`, `grade_research.py` |
| Check the running app | `verify_live_memory.py` |
| Export and package | `export.py`, `build_release_receipt.py`, `stage_release.py` |
| Build the report | `build_notebook.py` |

`prompt.py` reads the shared runtime templates; `structured.py` handles paired sources and constrained output. The bootstrap, refinement and research-v2 scripts belong to earlier experiments. `v1/` is the older 1.7B pipeline with a different schema and dataset.

Current results are in `data/research-v3/`, including `manifest.json`, `eval/`, `final-acceptance.json` and `release-receipt.json`. The final adapter is in `out/local-4b-general/`. Keep these files and their hashes when transferring an experiment.

## Training

General stage A used 585 audited rows. Stage B used 1,510 new rows plus 128 replay exposures, completing 205 optimizer steps. Together they cover 2,095 unique training rows.

This is the stage-B continuation command. It requires the saved stage-A adapter and frozen curriculum. Use a new output directory for a rerun; the saved `run-config.json` records the original settings.

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/train.py \
  --base Qwen/Qwen3-4B-Instruct-2507 \
  --revision cdbee75f17c01a7cc42f958dc650907174af0554 \
  --qlora --sets slm-distill/data/research-v3/continuation-sets \
  --adapter slm-distill/out/local-4b-general-stage-a/final \
  --out slm-distill/out/local-4b-general-rerun \
  --rank 16 --accum 8 --epochs 1 --max-length 4096 --loss-chunk-size 256 --lr 3e-5 --eval-on-epoch
```

Generating new labels uses authenticated Pi and provider credits. See each script's `--help` before running it. Changing prompts or regenerating inputs creates a different experiment.

## Evaluation

The internal test has 64 distinct episodes. The external diagnostic has sixteen LoCoMo windows kept out of training and prompt tuning. Compared models use Q8 quantization, the same decoder and a 12,288-token serving context. That context size does not show learned long-context ability.

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/evaluate_research.py \
  --model mem-extractor:general-v1 --label student-general-64 \
  --rows slm-distill/data/research-v3/fresh-evaluation-64.jsonl \
  --output-folder slm-distill/data/research-v3/eval
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/grade_research.py \
  --label student-general-64 --root slm-distill/data/research-v3
```

Use separate labels and output folders for new runs. Repeat for the base and external set as described in the protocol. Report support and coverage separately from structural validity, keeping failed windows in the denominator.

`verify_live_memory.py` checks the selected model through extraction, correction history, observations, graph projection, recall and cited answers in a fresh namespace.

## Export and release

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/export.py \
  --base Qwen/Qwen3-4B-Instruct-2507 \
  --revision cdbee75f17c01a7cc42f958dc650907174af0554 \
  --adapter slm-distill/out/local-4b-general/final \
  --name mem-extractor:general-v1 --quant q8_0
```

Export merges the adapter and converts it to Q8_0 GGUF. It needs about 13 GB of temporary space; `--work` selects the workspace. Keep the checksum and supplied non-thinking chat template.

After evaluation and live checks pass:

```bash
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/build_release_receipt.py
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/stage_release.py \
  --adapter slm-distill/out/local-4b-general/final \
  --gguf slm-distill/out/local-4b-general/mem-extractor-q8_0.gguf \
  --receipt slm-distill/data/research-v3/release-receipt.json \
  --out /path/to/empty-release-directory
slm-distill/release.sh /path/to/empty-release-directory --verify-only
```

The receipt builder checks weights, evaluations and live results. Staging copies only the listed public artifacts. Removing `--verify-only` uploads to a private staging repository; public publication is a separate step. The recorded public release matched all 22 staged hashes; checks are in `data/research-v3/huggingface-publication.json`.
