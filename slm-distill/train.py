"""Answer-only LoRA training for the three local memory-worker tasks.

The default 1.7B experiment uses a bf16 base. --base and --qlora support the
4B instruction-model experiment with NF4, double quantization and bf16 compute.
Training records exact inputs, length exclusions and adapter configuration.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
from pathlib import Path

import torch
import torch.nn.functional as F
from torch.utils.checkpoint import checkpoint
from datasets import Dataset
from peft import LoraConfig, PeftModel, get_peft_model, prepare_model_for_kbit_training
from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig, DataCollatorForSeq2Seq, Trainer, TrainingArguments

import prompt

HERE = Path(__file__).resolve().parent
BASE = "Qwen/Qwen3-1.7B"
MAXLEN = 8192


def encode(tok, path: Path, tasks: set[str] | None = None, max_length: int = MAXLEN, exclusions: list | None = None) -> tuple[Dataset, int]:
    rows, skipped = [], 0
    for line in path.read_text().splitlines():
        r = json.loads(line)
        if tasks and r["task"] not in tasks:
            continue
        text = prompt.chat_prompt(tok, r["prompt"])
        p_ids = tok(text, add_special_tokens=False)["input_ids"]
        t_ids = tok(r["target"] + "<|im_end|>", add_special_tokens=False)["input_ids"]
        if len(p_ids) + len(t_ids) > max_length:
            skipped += 1
            if exclusions is not None:
                exclusions.append({"timeline": r.get("timeline"), "task": r["task"], "tokens": len(p_ids)+len(t_ids), "prompt_sha256": hashlib.sha256(r["prompt"].encode()).hexdigest()})
            continue
        ids = p_ids + t_ids
        rows.append({"input_ids": ids, "labels": [-100] * len(p_ids) + t_ids, "attention_mask": [1] * len(ids)})
    return Dataset.from_list(rows), skipped


class AnswerOnlyTrainer(Trainer):
    """Cross entropy over the answer tokens only.

    The default loss builds logits for every position: 8,000 tokens by a 152,000 word vocabulary
    is 5 GB in float32, for a prompt whose loss is masked anyway. Here the output head is applied
    only where there is a label.
    """

    def compute_loss(self, model, inputs, return_outputs=False, num_items_in_batch=None):
        labels = inputs["labels"]
        inner = model.base_model.model  # Qwen3ForCausalLM under the PEFT wrapper
        hidden = inner.model(input_ids=inputs["input_ids"], attention_mask=inputs["attention_mask"]).last_hidden_state
        shifted_labels = labels[:, 1:]
        keep = shifted_labels != -100
        answer_hidden, answer_labels = hidden[:, :-1][keep], shifted_labels[keep]
        # A long answer still creates gigabytes of vocabulary logits and their
        # backward buffers. Recompute small chunks during backward instead of
        # keeping every logit alive; the summed token loss stays identical.
        def token_loss(states, targets):
            return F.cross_entropy(inner.lm_head(states.to(inner.lm_head.weight.dtype)).float(), targets, reduction="sum")

        losses = []
        chunk_size = getattr(self, "loss_chunk_size", 128)
        for start in range(0, len(answer_labels), chunk_size):
            states, targets = answer_hidden[start:start + chunk_size], answer_labels[start:start + chunk_size]
            losses.append(checkpoint(token_loss, states, targets, use_reentrant=False)
                          if torch.is_grad_enabled() else token_loss(states, targets))
        total = torch.stack(losses).sum()
        loss = total / (num_items_in_batch if num_items_in_batch is not None else keep.sum().clamp(min=1))
        return (loss, None) if return_outputs else loss

    def prediction_step(self, model, inputs, prediction_loss_only, ignore_keys=None):
        # Evaluation only needs the loss on the answer tokens, not logits for every position.
        with torch.no_grad():
            loss = self.compute_loss(model, inputs)
        return loss.detach(), None, None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--eval-on-epoch", action="store_true")
    ap.add_argument("--loss-chunk-size", type=int, default=128)
    ap.add_argument("--max-length", type=int, default=MAXLEN)
    ap.add_argument("--base", default=BASE)
    ap.add_argument("--revision")
    ap.add_argument("--qlora", action="store_true")
    ap.add_argument("--sets", type=Path, default=HERE / "data" / "sets")
    ap.add_argument("--out", type=Path, default=HERE / "out" / "worker-lora")
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--adapter", type=Path, help="continue training a previously saved LoRA adapter")
    ap.add_argument("--accum", type=int, default=16)
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--rank", type=int, default=64)
    ap.add_argument("--max-steps", type=int, default=-1, help="stop early, for a smoke run")
    ap.add_argument("--limit", type=int, help="use only the first N training rows")
    args = ap.parse_args()

    assert args.loss_chunk_size > 0, "loss chunk size must be positive"
    assert torch.cuda.is_available(), "no CUDA device visible"
    print(f"gpu: {torch.cuda.get_device_name(0)}  torch {torch.__version__}")
    tok = AutoTokenizer.from_pretrained(args.base, revision=args.revision)
    exclusions = {"train": [], "val": []}
    train_ds, skipped_t = encode(tok, args.sets / "train.jsonl", max_length=args.max_length, exclusions=exclusions["train"])
    val_ds, skipped_v = encode(tok, args.sets / "val.jsonl", max_length=args.max_length, exclusions=exclusions["val"])
    args.out.mkdir(parents=True, exist_ok=True)
    receipt = {"arguments": {k: str(v) if isinstance(v, Path) else v for k,v in vars(args).items()},
               "counts": {"train": len(train_ds), "val": len(val_ds)}, "exclusions": exclusions,
               "system_sha256": hashlib.sha256(prompt.SYSTEM.encode()).hexdigest(),
               "extraction_template_sha256": hashlib.sha256(prompt.EXTRACT_TEMPLATE.encode()).hexdigest(),
               "dataset_sha256": {split: hashlib.sha256((args.sets/f"{split}.jsonl").read_bytes()).hexdigest() for split in ("train", "val")}}
    (args.out/"run-config.json").write_text(json.dumps(receipt, indent=2)+"\n")
    if args.limit:
        train_ds = train_ds.select(range(min(args.limit, len(train_ds))))
    lengths = [len(x) for x in train_ds["input_ids"]]
    receipt["tokens"] = {"train_total": sum(lengths), "train_answer": sum(sum(t != -100 for t in labels) for labels in train_ds["labels"]), "mean": sum(lengths)/len(lengths), "max": max(lengths)}
    receipt["counts"]["actual_train_after_limit"] = len(train_ds)
    (args.out/"run-config.json").write_text(json.dumps(receipt, indent=2)+"\n")
    print(f"train {len(train_ds)} (skipped {skipped_t} over {args.max_length} tokens)  val {len(val_ds)} (skipped {skipped_v})  "
          f"tokens/example mean {sum(lengths)/len(lengths):.0f} max {max(lengths)}")

    quantization = BitsAndBytesConfig(load_in_4bit=True, bnb_4bit_quant_type="nf4", bnb_4bit_use_double_quant=True, bnb_4bit_compute_dtype=torch.bfloat16) if args.qlora else None
    model = AutoModelForCausalLM.from_pretrained(args.base, revision=args.revision, dtype=torch.bfloat16, device_map={"": 0}, attn_implementation="sdpa", quantization_config=quantization)
    if args.qlora:
        model = prepare_model_for_kbit_training(model, gradient_checkpointing_kwargs={"use_reentrant": False})
        # Keep frozen embedding/head in bf16; PEFT casts them to fp32 by default.
        model.model.embed_tokens.to(torch.bfloat16)
        model.lm_head.to(torch.bfloat16)
    model.config.use_cache = False
    model.enable_input_require_grads()
    if args.adapter:
        model = PeftModel.from_pretrained(model, str(args.adapter), is_trainable=True)
    else:
        model = get_peft_model(model, LoraConfig(
            r=args.rank, lora_alpha=2 * args.rank, lora_dropout=0.05, bias="none", task_type="CAUSAL_LM",
            target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"]))
    model.print_trainable_parameters()

    steps_per_epoch = max(1, len(train_ds) // args.accum)
    trainer = AnswerOnlyTrainer(
        model=model,
        args=TrainingArguments(
            output_dir=str(args.out), num_train_epochs=args.epochs, max_steps=args.max_steps,
            per_device_train_batch_size=1, gradient_accumulation_steps=args.accum,
            learning_rate=args.lr, lr_scheduler_type="cosine", warmup_steps=max(3, steps_per_epoch // 20),
            logging_steps=5, eval_strategy=("epoch" if args.eval_on_epoch else "steps") if len(val_ds) else "no", eval_steps=max(10, steps_per_epoch // 2),
            per_device_eval_batch_size=1, save_strategy="epoch", save_total_limit=2, save_only_model=True,
            bf16=True, gradient_checkpointing=True, gradient_checkpointing_kwargs={"use_reentrant": False},
            optim="adamw_torch_fused", max_grad_norm=1.0, weight_decay=0.0, report_to=[], seed=13,
        ),
        train_dataset=train_ds, eval_dataset=val_ds if len(val_ds) else None,
        data_collator=DataCollatorForSeq2Seq(tok, padding=True, label_pad_token_id=-100),
    )
    trainer.loss_chunk_size = args.loss_chunk_size
    result = trainer.train()
    trainer.save_state()
    (args.out/"training-result.json").write_text(json.dumps({"metrics": result.metrics, "global_step": trainer.state.global_step, "peak_vram_bytes": torch.cuda.max_memory_allocated(), "loss_chunk_size": args.loss_chunk_size}, indent=2)+"\n")
    final = args.out / "final"
    model.save_pretrained(final)
    tok.save_pretrained(final)
    print(f"adapter saved: {final}  peak vram {torch.cuda.max_memory_allocated()/1e9:.1f} GB")


if __name__ == "__main__":
    main()
