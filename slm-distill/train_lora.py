"""QLoRA finetune of Qwen3-1.7B on teacher-generated extraction labels.

Student runs locally on the RTX 5070 Ti (16 GB, Blackwell sm_120). 4-bit NF4 base
+ LoRA adapters keeps peak VRAM near 4-5 GB with a 2048-token sequence.

  python train_lora.py --epochs 2
"""

import argparse
import json
from pathlib import Path

import torch
from datasets import Dataset
from peft import LoraConfig, get_peft_model, prepare_model_for_kbit_training
from transformers import (
    AutoModelForCausalLM,
    AutoTokenizer,
    BitsAndBytesConfig,
    DataCollatorForSeq2Seq,
    Trainer,
    TrainingArguments,
)

from schema import SYSTEM_PROMPT, build_user_prompt

ROOT = Path(__file__).parent
BASE = "Qwen/Qwen3-1.7B"
MAXLEN = 2048


def build_dataset(tok, path: Path, split: str) -> Dataset:
    """Mask the prompt; train only on the JSON the student must produce."""
    rows = []
    for line in path.read_text().splitlines():
        r = json.loads(line)
        if r.get("split") != split:
            continue
        prompt = tok.apply_chat_template(
            [
                {"role": "system", "content": SYSTEM_PROMPT},
                {
                    "role": "user",
                    "content": build_user_prompt(
                        r["session_date"], r["speakers"], r["turns"]
                    ),
                },
            ],
            tokenize=False,
            add_generation_prompt=True,
            enable_thinking=False,
        )
        # compact JSON: fewer tokens to learn, and it is what the Rust side parses
        target = json.dumps(r["target"], separators=(",", ":")) + tok.eos_token

        p_ids = tok(prompt, add_special_tokens=False)["input_ids"]
        t_ids = tok(target, add_special_tokens=False)["input_ids"]
        ids = (p_ids + t_ids)[:MAXLEN]
        if len(ids) < len(p_ids) + 4:  # target got truncated away entirely
            continue
        labels = ([-100] * len(p_ids) + t_ids)[:MAXLEN]
        rows.append(
            {"input_ids": ids, "labels": labels, "attention_mask": [1] * len(ids)}
        )
    return Dataset.from_list(rows)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="data/silver.jsonl")
    ap.add_argument("--out", default="out/qwen3-1.7b-memex-lora")
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--bsz", type=int, default=1)
    ap.add_argument("--accum", type=int, default=8)
    ap.add_argument("--lr", type=float, default=1e-4)
    args = ap.parse_args()

    assert torch.cuda.is_available(), "no CUDA device visible"
    cap = torch.cuda.get_device_capability()
    print(
        f"gpu: {torch.cuda.get_device_name(0)}  sm_{cap[0]}{cap[1]}  torch {torch.__version__}"
    )

    tok = AutoTokenizer.from_pretrained(BASE)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token

    train_ds = build_dataset(tok, ROOT / args.data, "train")
    val_ds = build_dataset(tok, ROOT / args.data, "val")
    print(f"train {len(train_ds)}  val {len(val_ds)}")

    model = AutoModelForCausalLM.from_pretrained(
        BASE,
        quantization_config=BitsAndBytesConfig(
            load_in_4bit=True,
            bnb_4bit_quant_type="nf4",
            bnb_4bit_use_double_quant=True,
            bnb_4bit_compute_dtype=torch.bfloat16,
        ),
        dtype=torch.bfloat16,
        device_map={"": 0},
        attn_implementation="sdpa",
    )
    model.config.use_cache = False
    model = prepare_model_for_kbit_training(model, use_gradient_checkpointing=True)
    model = get_peft_model(
        model,
        LoraConfig(
            r=32,
            lora_alpha=64,
            lora_dropout=0.05,
            bias="none",
            task_type="CAUSAL_LM",
            target_modules=[
                "q_proj",
                "k_proj",
                "v_proj",
                "o_proj",
                "gate_proj",
                "up_proj",
                "down_proj",
            ],
        ),
    )
    model.print_trainable_parameters()

    trainer = Trainer(
        model=model,
        args=TrainingArguments(
            output_dir=str(ROOT / args.out),
            num_train_epochs=args.epochs,
            per_device_train_batch_size=args.bsz,
            gradient_accumulation_steps=args.accum,
            learning_rate=args.lr,
            lr_scheduler_type="cosine",
            # transformers v5 dropped warmup_ratio. 642 examples / 8 accum = 80
            # steps per epoch, 160 total; 5 steps is the same ~3% warmup.
            warmup_steps=5,
            logging_steps=5,
            eval_strategy="steps" if len(val_ds) else "no",
            eval_steps=50,
            per_device_eval_batch_size=1,
            save_strategy="epoch",
            save_total_limit=2,
            bf16=True,
            gradient_checkpointing=True,
            gradient_checkpointing_kwargs={"use_reentrant": False},
            optim="paged_adamw_8bit",
            max_grad_norm=0.3,
            report_to=[],
            seed=13,
        ),
        train_dataset=train_ds,
        eval_dataset=val_ds if len(val_ds) else None,
        data_collator=DataCollatorForSeq2Seq(
            tok, padding=True, label_pad_token_id=-100
        ),
    )
    trainer.train()

    final = ROOT / args.out / "final"
    model.save_pretrained(final)
    tok.save_pretrained(final)
    print(f"\nadapter saved: {final}")
    print(f"peak vram: {torch.cuda.max_memory_allocated()/1e9:.1f} GB")


if __name__ == "__main__":
    main()
