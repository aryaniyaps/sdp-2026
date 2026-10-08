# Local setup receipt

Configured on 2026-10-08 on this Ubuntu host. This file describes the installed local environment; model evaluation results are recorded separately below.

## Open the project

- Pi console and graph: http://127.0.0.1:18089/
- Native Memory Engine API: http://127.0.0.1:8080/
- Browser demo engine API: http://127.0.0.1:18080/
- Ollama: http://127.0.0.1:11434/

The browser console uses Pi's refreshed `openai` login and `gpt-5.6-sol`. The host global Pi default is preserved. Run `pi` and `/login` if that login expires. Teacher generation uses the same provider with tools disabled. Credentials stay outside the repository.

To use the same browser memory from the host CLI, select the refreshed provider explicitly:

```sh
MEMORY_URL=http://127.0.0.1:18080 MEMORY_NAMESPACE=user:dev pi --provider openai --model gpt-5.6-sol -e integrations/pi/extension.ts
```

The native API is an enabled user service:

```sh
systemctl --user status sdp-2026-memory
systemctl --user restart sdp-2026-memory
journalctl --user -u sdp-2026-memory -n 50
```

Start the complete browser stack with its preserved PostgreSQL/Neo4j volumes:

```sh
~/.local/state/sdp-hosted-local/start-demo.sh
```

Generated browser configuration lives under `~/.local/state/sdp-hosted-local/`; the native API reads the ignored, private `.env`. Docker bridges use explicit NetworkManager exclusions. The browser network is `10.253.202.0/24` to avoid the host LAN.

## Installed development and training tools

Rust 1.96.1 with rustfmt/clippy, Node dependencies for frontend and Pi integration, Playwright Chromium, Ollama, pgvector PostgreSQL and Neo4j containers, Pi 1.0.2, benchmark datasets, Python 3.12 training virtualenv, CUDA PyTorch, transformers/PEFT, and the checksum-verified pinned llama.cpp converter.

```sh
source slm-distill/.venv/bin/activate
python -c 'import torch; print(torch.__version__, torch.cuda.get_device_name(0))'
python -m pytest slm-distill -q
```

The host ROS `PYTHONPATH` must be cleared for this environment; its activation script does that. With direct virtualenv commands use `env -u PYTHONPATH`.

## General-purpose replacement worker

The configured `slm-v1` release was unavailable. The first replacement used Qwen3-1.7B with deterministic examples followed by teacher-labelled conversations. Its strong synthetic results did not establish general-purpose accuracy. Subsequent experiments use Qwen3-4B-Instruct-2507, broader everyday-memory conversations, source-grounded extraction, label audits and separate held-out evaluations. Coding conversations are one application of the memory engine, not its scope.

Both running services now use `mem-extractor:general-v1` (Qwen3-4B-Instruct-2507 with the trained adapter, Q8_0), context 12,288 and output cap 3,072. The default `mem-extractor` alias points to the same verified GGUF (`5f3d769b350655b4dc316953cbd362c92fa573ae3aabec5c77725f19cf6f0e36`). The older `memex-extractor:local-v1-cpu` alias remains available for rollback. This is an experimental multitask release: held-out extraction regressed against the base model, while consolidation and reflection improved. The executed [iteration notebook](slm-distill/Finetuning_Iteration_Report.ipynb) records measured results and limitations.

Adapters, provenance and trainer states: `slm-distill/out/` (ignored).
Datasets, teacher cache and evaluation receipts: `slm-distill/data/` (ignored).

The following commands reproduce the earlier 1.7B experiments, not the final 4B release:

```sh
python slm-distill/bootstrap_dataset.py
PI_TEACHER_COMPACT=1 python slm-distill/run_pi_corpus.py
python slm-distill/train.py --sets slm-distill/data/bootstrap-sets --out slm-distill/out/local-bootstrap --epochs 2
python slm-distill/refine_local.py
python slm-distill/export.py --adapter slm-distill/out/local-refined/final --name memex-extractor --quant q8_0
python slm-distill/evaluate_bootstrap.py --model memex-extractor
python slm-distill/evaluate_local.py --model memex-extractor:local-v1 --label local-v1
```

Training and inference share an 8 GB GPU. Both backends currently use `qwen3-embedding-cpu`, an alias with `num_gpu 0`, so embedding calls do not interfere with training. Worker GPU inference must also be unloaded before training. Worker generation explicitly sets `think: false` to match the no-thinking training format.

The original fine-tuned model has not been recovered. This replacement must be assessed from its own evaluation receipts. Synthetic schema acceptance is not a claim of real conversation accuracy.

## Verify the deployed memory engine

The live check creates its own namespace and preserves its receipts. It exercises everyday preferences and an explicit sister relationship, a home-city correction, supported derived observations, Neo4j projection, recall and cited reflection. It requires actual source quotes, checks that the old city is not still active, and rejects the unsupported addition of a country absent from the fixture. It fails when a required check fails; merely accepting a retain request is insufficient.

```sh
env -u PYTHONPATH slm-distill/.venv/bin/python slm-distill/verify_live_memory.py --url http://127.0.0.1:18080
```

The default receipt is `slm-distill/data/research-v3/final-acceptance.json`. Check its `deployment_identity_before.health.worker_model`, `passed`, per-check outcomes and timestamps before interpreting it as evidence for a particular release. PostgreSQL remains authoritative; Neo4j is the derived graph projection. Extraction and consolidation are asynchronous, so inspect `/api/v2/jobs?namespace=YOUR_NAMESPACE` until they finish.

The final native acceptance passed all 16 checks. Its first reflection attempt exposed an invalid citation UUID; the deployed decoder now constrains citations to supplied evidence IDs. The second inference run passed after correcting a harness attribution false negative (resolved subject Maya Sen); `final-acceptance.json` explicitly records that same-output re-audit rather than claiming fresh inference. Synthetic screenshots are `slm-distill/data/research-v3/final-graph.png` and `final-graph-observations.png`.

The original browser `user:dev` queue also completed: 43 extraction, 43 consolidation and 160 projection jobs succeeded, with no pending or failed jobs. Eight active derived observations each have at least two distinct supporting facts. The Neo4j projection contains 112 nodes and 197 relationships, including 21 support/derivation edges. All 142 source-quote links match their stored source chunks verbatim. These are a point-in-time recovery receipt (`original-queue-final.json`), not a general semantic-accuracy benchmark.

Release repository: [aryaniyaps/mem-extractor](https://huggingface.co/aryaniyaps/mem-extractor). The installer pins revision `77851da91c15fd4e20018a1b0ace13d29e7eb635`, verifies the published SHA-256 manifest, and creates `mem-extractor`; set `SLM_REVISION` explicitly to select another revision.

```sh
scripts/fetch-slm.sh
```
