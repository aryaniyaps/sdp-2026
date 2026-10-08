"""LoRA fine-tune of Qwen3-1.7B on the teacher's labels for the three worker tasks.

The base is kept in bf16 (not 4-bit): the adapter is merged into a bf16 model for serving, so
training against the same weights avoids a quantization mismatch. Loss is on the answer only.

    python train.py --epochs 2
"""

from __future__ import annotations

import argparse
import json
import random
from pathlib import Path

import torch
import torch.nn.functional as F
from datasets import Dataset
from peft import LoraConfig, get_peft_model
from transformers import AutoModelForCausalLM, AutoTokenizer, DataCollatorForSeq2Seq, Trainer, TrainingArguments

import prompt

HERE = Path(__file__).resolve().parent
BASE = "Qwen/Qwen3-1.7B"
MAXLEN = 8192


def encode(tok, path: Path, tasks: set[str] | None = None) -> tuple[Dataset, int]:
    rows, skipped = [], 0
    for line in path.read_text().splitlines():
        r = json.loads(line)
        if tasks and r["task"] not in tasks:
            continue
        text = prompt.chat_prompt(tok, r["prompt"])
        p_ids = tok(text, add_special_tokens=False)["input_ids"]
        t_ids = tok(r["target"] + "<|im_end|>", add_special_tokens=False)["input_ids"]
        if len(p_ids) + len(t_ids) > MAXLEN:
            skipped += 1
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
        logits = inner.lm_head(hidden[:, :-1][keep]).float()
        total = F.cross_entropy(logits, shifted_labels[keep], reduction="sum")
        loss = total / (num_items_in_batch if num_items_in_batch is not None else keep.sum().clamp(min=1))
        return (loss, None) if return_outputs else loss

    def prediction_step(self, model, inputs, prediction_loss_only, ignore_keys=None):
        # Evaluation only needs the loss on the answer tokens, not logits for every position.
        with torch.no_grad():
            loss = self.compute_loss(model, inputs)
        return loss.detach(), None, None


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--sets", type=Path, default=HERE / "data" / "sets")
    ap.add_argument("--out", type=Path, default=HERE / "out" / "worker-lora")
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--accum", type=int, default=16)
    ap.add_argument("--lr", type=float, default=2e-4)
    ap.add_argument("--rank", type=int, default=64)
    ap.add_argument("--max-steps", type=int, default=-1, help="stop early, for a smoke run")
    ap.add_argument("--limit", type=int, help="use only the first N training rows")
    args = ap.parse_args()

    assert torch.cuda.is_available(), "no CUDA device visible"
    print(f"gpu: {torch.cuda.get_device_name(0)}  torch {torch.__version__}")
    tok = AutoTokenizer.from_pretrained(BASE)
    train_ds, skipped_t = encode(tok, args.sets / "train.jsonl")
    val_ds, skipped_v = encode(tok, args.sets / "val.jsonl")
    if args.limit:
        train_ds = train_ds.select(range(min(args.limit, len(train_ds))))
    lengths = [len(x) for x in train_ds["input_ids"]]
    print(f"train {len(train_ds)} (skipped {skipped_t} over {MAXLEN} tokens)  val {len(val_ds)} (skipped {skipped_v})  "
          f"tokens/example mean {sum(lengths)/len(lengths):.0f} max {max(lengths)}")

    model = AutoModelForCausalLM.from_pretrained(BASE, dtype=torch.bfloat16, device_map={"": 0}, attn_implementation="sdpa")
    model.config.use_cache = False
    model.enable_input_require_grads()
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
            logging_steps=5, eval_strategy="steps" if len(val_ds) else "no", eval_steps=max(10, steps_per_epoch // 2),
            per_device_eval_batch_size=1, save_strategy="epoch", save_total_limit=2,
            bf16=True, gradient_checkpointing=True, gradient_checkpointing_kwargs={"use_reentrant": False},
            optim="adamw_torch_fused", max_grad_norm=1.0, weight_decay=0.0, report_to=[], seed=13,
        ),
        train_dataset=train_ds, eval_dataset=val_ds if len(val_ds) else None,
        data_collator=DataCollatorForSeq2Seq(tok, padding=True, label_pad_token_id=-100),
    )
    trainer.train()
    final = args.out / "final"
    model.save_pretrained(final)
    tok.save_pretrained(final)
    print(f"adapter saved: {final}  peak vram {torch.cuda.max_memory_allocated()/1e9:.1f} GB")


if __name__ == "__main__":
    main()
