"""Score saved generations under several matching rules.

The strict rule (exact type + subject::predicate string) is unusable as a headline
number: the predicate is free-form model text, so "sam::likes" and "sam::loves"
describing the same fact score as a total miss. It is kept as a lower bound.

The reported metric is RELAXED: same type, same canonical subject, and content-word
F1 >= 0.5 between the two statements, matched greedily best-first. Transparent and
explainable - no embedding model, no hidden threshold tuning.

  python score.py
"""

import json
import re
from pathlib import Path

STOP = {
    "a",
    "an",
    "the",
    "is",
    "are",
    "was",
    "were",
    "to",
    "of",
    "in",
    "on",
    "at",
    "for",
    "with",
    "and",
    "or",
    "his",
    "her",
    "their",
    "its",
    "he",
    "she",
    "they",
    "that",
    "this",
    "has",
    "have",
    "had",
    "be",
    "been",
    "as",
    "by",
}


def words(s):
    return {w for w in re.findall(r"[a-z0-9]+", (s or "").lower()) if w not in STOP}


def tokf1(a, b):
    A, B = words(a), words(b)
    if not A or not B:
        return 0.0
    i = len(A & B)
    if not i:
        return 0.0
    p, r = i / len(A), i / len(B)
    return 2 * p * r / (p + r)


def subj(m):
    return re.sub(r"[^a-z0-9]", "", (m.get("subject") or "").lower())


def strict_pairs(mems):
    return {(m.get("type"), m.get("entity_key")) for m in mems if m.get("entity_key")}


def relaxed_match(pred, gold, thresh=0.5, require_type=True):
    """Greedy best-first bipartite matching on statement content-word F1."""
    cands = []
    for i, p in enumerate(pred):
        for j, g in enumerate(gold):
            if subj(p) != subj(g):
                continue
            if require_type and p.get("type") != g.get("type"):
                continue
            sc = tokf1(p.get("statement"), g.get("statement"))
            if sc >= thresh:
                cands.append((sc, i, j))
    cands.sort(reverse=True)
    up, ug, n = set(), set(), 0
    for sc, i, j in cands:
        if i in up or j in ug:
            continue
        up.add(i)
        ug.add(j)
        n += 1
    return n


def prf(tp, npred, ngold):
    p = tp / npred if npred else (1.0 if not ngold else 0.0)
    r = tp / ngold if ngold else 1.0
    return p, r, (2 * p * r / (p + r) if p + r else 0.0)


def score(rows, mode):
    P = R = F = 0.0
    for row in rows:
        pred, gold = row["pred"] or [], row["gold"]
        if mode == "strict":
            sp, sg = strict_pairs(pred), strict_pairs(gold)
            tp, npred, ngold = len(sp & sg), len(sp), len(sg)
        elif mode == "relaxed":
            tp = relaxed_match(pred, gold, 0.5, True)
            npred, ngold = len(pred), len(gold)
        elif mode == "relaxed_notype":
            tp = relaxed_match(pred, gold, 0.5, False)
            npred, ngold = len(pred), len(gold)
        elif mode == "subject":
            sp = {subj(m) for m in pred if subj(m)}
            sg = {subj(m) for m in gold if subj(m)}
            tp, npred, ngold = len(sp & sg), len(sp), len(sg)
        p, r, f = prf(tp, npred, ngold)
        P += p
        R += r
        F += f
    n = len(rows)
    return P / n, R / n, F / n


def report():
    MODES = [
        ("strict", "exact type + subject::predicate string"),
        ("relaxed", "same type + subject, statement overlap >= 0.5"),
        ("relaxed_notype", "same subject, statement overlap >= 0.5 (type ignored)"),
        ("subject", "did it find the same entities at all"),
    ]

    files = sorted(Path("data").glob("gen_*.json"))
    if not files:
        raise SystemExit("no data/gen_*.json - run eval.py first")

    print(f"{'variant':<16}{'mode':<17}{'prec':>8}{'rec':>8}{'F1':>8}")
    print("-" * 57)
    store = {}
    for f in files:
        tag = f.stem.replace("gen_", "")
        rows = json.loads(f.read_text())
        store[tag] = {}
        for mode, _ in MODES:
            p, r, fl = score(rows, mode)
            store[tag][mode] = fl
            print(f"{tag:<16}{mode:<17}{p:>8.3f}{r:>8.3f}{fl:>8.3f}")
        print()

    print("what each mode means:")
    for mode, desc in MODES:
        print(f"  {mode:<17} {desc}")

    if "distilled" in store and "base-zeroshot" in store:
        print("\ndistillation gain by mode:")
        for mode, _ in MODES:
            d = store["distilled"][mode] - store["base-zeroshot"][mode]
            print(
                f"  {mode:<17} {store['base-zeroshot'][mode]:.3f} -> "
                f"{store['distilled'][mode]:.3f}   ({d:+.3f})"
            )


if __name__ == "__main__":
    report()
