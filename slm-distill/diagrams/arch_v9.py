"""System architecture as a block diagram.

Corrections this version answers:
  the fine-tuned SLM is a component with its own box, not a phrase
  every arrow is labelled with the data that travels along it
  each stage carries the rule it applies, where a rule applies
  boxes and arrows only

    python diagrams/arch_v9.py
    rsvg-convert -w 3200 -h 1800 diagrams/arch_v9.svg -o diagrams/arch_v9.png
"""

from pathlib import Path

W, H = 3200, 1800
INK, BODY, LAB = "#111111", "#2E2E2E", "#5C5C5C"
RULE, SLM_BG, STORE_BG = "#000000", "#E7EDF8", "#F2F2F0"

SERIF = "Georgia, 'Times New Roman', serif"
SANS = "Helvetica, Arial, sans-serif"
MONO = "'DejaVu Sans Mono', Consolas, monospace"

o = []
def e(s): o.append(s)
def esc(s): return str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def t(x, y, s, size=27, fill=BODY, weight="400", fam=SANS, anchor="start"):
    e(f'<text x="{x}" y="{y}" font-family="{fam}" font-size="{size}" font-weight="{weight}" '
      f'fill="{fill}" text-anchor="{anchor}">{esc(s)}</text>')


def lines(x, y, ls, size=26, fill=BODY, lh=33, fam=SANS, weight="400", anchor="start"):
    for i, ln in enumerate(ls):
        t(x, y + i * lh, ln, size, fill, weight, fam, anchor)


M = 74
N = 5
GAP = 252
BOXW = (W - 2 * M - (N - 1) * GAP) / N          # 408
BOXH = 372


def stage(x, y, num, name, body, rule, slm=False, mod=None):
    e(f'<rect x="{x}" y="{y}" width="{BOXW}" height="{BOXH}" fill="'
      f'{SLM_BG if slm else "#FFFFFF"}" stroke="{RULE}" stroke-width="{4 if slm else 2.5}"/>')
    e(f'<line x1="{x}" y1="{y+64}" x2="{x+BOXW}" y2="{y+64}" stroke="{RULE}" '
      f'stroke-width="{4 if slm else 2.5}"/>')
    t(x + 18, y + 44, num, 26, LAB, "700", MONO)
    t(x + 62, y + 44, name, 33, INK, "700", SANS)
    if mod:
        t(x + BOXW - 18, y + 44, mod, 24, LAB, "700", MONO, anchor="end")
    by = y + 104
    if slm:
        e(f'<rect x="{x+18}" y="{y+80}" width="248" height="34" fill="{RULE}"/>')
        t(x + 30, y + 105, "FINE-TUNED SLM", 22, "#FFFFFF", "700", SANS)
        by = y + 148
    lines(x + 18, by, body, 25, BODY, 32)
    if rule:
        ry = y + BOXH - 26 - 30 * len(rule)
        e(f'<line x1="{x+18}" y1="{ry-26}" x2="{x+BOXW-18}" y2="{ry-26}" '
          f'stroke="{LAB}" stroke-width="1.5"/>')
        lines(x + 18, ry + 6, rule, 24, INK, 30, MONO)


def flow_arrow(x1, x2, y, label):
    """Horizontal arrow between two stages, carrying its artefact."""
    e(f'<path d="M{x1} {y} L{x2-10} {y}" stroke="{RULE}" stroke-width="3" '
      f'marker-end="url(#ar)"/>')
    mid = (x1 + x2) / 2
    lines(mid, y - 74 + (2 - len(label)) * 15, label, 22, INK, 30, MONO, anchor="middle")


def down_arrow(x, y1, y2, label, side=1):
    e(f'<path d="M{x} {y1} L{x} {y2-10}" stroke="{RULE}" stroke-width="3" '
      f'marker-end="url(#ar)"/>')
    t(x + 20 * side, (y1 + y2) / 2 + 8, label, 24, INK, "400", MONO,
      anchor="start" if side > 0 else "end")


e(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">')
e('<defs><marker id="ar" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" '
  f'markerHeight="8" orient="auto"><path d="M0 0 L10 5 L0 10 z" fill="{RULE}"/></marker></defs>')
e(f'<rect width="{W}" height="{H}" fill="#FFFFFF"/>')

t(M, 74, "System Architecture", 52, INK, "700", SERIF)
e(f'<line x1="{M}" y1="104" x2="{W-M}" y2="104" stroke="{RULE}" stroke-width="2.5"/>')

# ── write path ────────────────────────────────────────────────────────────
WY = 210
t(M, WY - 26, "WRITE PATH", 30, INK, "700", SANS)
t(M + 240, WY - 26, "an interaction becomes a stored, versioned memory", 27, LAB)

WRITE = [
    ("01", "Receive", ["Validate the request,", "assign a request id,", "split text into chunks"],
     ["1 event \u2192 N chunks"]),
    ("02", "Extract", ["Fine-tuned SLM reads one", "chunk and emits a typed", "claim under a fixed grammar",
                       "Qwen3-1.7B, QLoRA, 4-bit"],
     ["c = exp(mean log p)", "accept if c \u2265 \u03c4"]),
    ("03", "Identify", ["Reduce subject and", "predicate to one canonical", "key, then look it up"],
     ["key = norm(subj)", "      + norm(pred)"]),
    ("04", "Version", ["If the value changed,", "close the old row and", "open a new one, linked"],
     ["active(v) iff", "v.valid_to is NULL"]),
    ("05", "Commit", ["Write every row in one", "transaction, or write", "nothing at all"],
     ["all rows or none"]),
]
WLAB = [
    ["chunk text,", "session id"],
    ["typed claim:", "subj, pred, obj", "+ confidence"],
    ["canonical key,", "memory id"],
    ["version row,", "supersedes edge"],
]
WMOD = ["M1", "M2", "M1", "M1", "M1"]
xs = [M + i * (BOXW + GAP) for i in range(N)]
for i, (num, name, body, rule) in enumerate(WRITE):
    stage(xs[i], WY, num, name, body, rule, slm=(i == 1), mod=WMOD[i])
    if i < N - 1:
        flow_arrow(xs[i] + BOXW, xs[i + 1], WY + BOXH / 2, WLAB[i])

# ── store ─────────────────────────────────────────────────────────────────
SY, SH = 760, 178
e(f'<rect x="{M}" y="{SY}" width="{W-2*M}" height="{SH}" fill="{STORE_BG}" '
  f'stroke="{RULE}" stroke-width="3"/>')
t(M + 26, SY + 54, "PostgreSQL 17 with pgvector", 34, INK, "700", SANS)
lines(M + 26, SY + 100, [
    "raw_events, chunks, memories, memory_versions, relations, traces",
    "inverted index and vector index over the same rows, history never deleted"],
    26, BODY, 34, MONO)
t(M + 1560, SY + 54, "M1  Service and persistence", 25, BODY, "400", SANS)
t(M + 1560, SY + 88, "M2  Fine-tuned SLM extraction", 25, BODY, "400", SANS)
t(M + 1560, SY + 122, "M3  Hybrid retrieval", 25, BODY, "400", SANS)
t(M + 1980, SY + 54, "M4  Maintenance and evaluation", 25, BODY, "400", SANS)
t(M + 1980, SY + 88, "observes every stage above", 25, LAB, "400", SANS)
t(W - M - 26, SY + 62, "", 26, INK, "700", SANS, anchor="end")
t(W - M - 26, SY + 54, "one active version per claim", 25, INK, "700", SANS, anchor="end")
t(W - M - 26, SY + 88, "every row keeps its source chunk", 25, INK, "700", SANS, anchor="end")
t(W - M - 26, SY + 122, "history is never deleted", 25, INK, "700", SANS, anchor="end")

down_arrow(xs[4] + BOXW / 2, WY + BOXH, SY, "committed rows")

# ── read path ─────────────────────────────────────────────────────────────
RY = 1094
down_arrow(xs[1] + BOXW / 2, SY + SH, RY, "live rows only", side=1)
t(M, RY - 26, "READ PATH", 30, INK, "700", SANS)
t(M + 226, RY - 26, "a question becomes cited evidence", 27, LAB)

READ = [
    ("06", "Interpret", ["Build a text query and", "embed the question,", "read session and time"],
     ["q = E(question)", "E: text \u2192 R\u00b9\u2070\u00b2\u2074"]),
    ("07", "Fetch", ["Search the same rows two", "ways at once: words by", "index, meaning by vector"],
     ["cos(q,d) = q.d /", "  (||q|| ||d||)"]),
    ("08", "Filter", ["Drop every row whose", "validity window has", "already closed"],
     ["keep d iff", "d.valid_to is NULL"]),
    ("09", "Fuse", ["Combine both rankings so", "agreement between them", "outranks either alone"],
     ["RRF(d) =", "\u03a3 1/(k + r\u1d62(d)), k=60"]),
    ("10", "Pack", ["Add evidence in rank", "order until the token", "budget is reached"],
     ["max \u03a3 rel(e)", "s.t. \u03a3 tok(e) \u2264 B"]),
]
RLAB = [
    ["text query,", "1024-d vector"],
    ["lexical top 20,", "semantic top 20"],
    ["live candidates", "only"],
    ["ranked list", "with scores"],
]
for i, (num, name, body, rule) in enumerate(READ):
    stage(xs[i], RY, num, name, body, rule, mod="M3")
    if i < N - 1:
        flow_arrow(xs[i] + BOXW, xs[i + 1], RY + BOXH / 2, RLAB[i])

# ── final output ──────────────────────────────────────────────────────────
OY = RY + BOXH + 66
e(f'<path d="M{xs[4]+BOXW/2} {RY+BOXH} L{xs[4]+BOXW/2} {OY-10}" stroke="{RULE}" '
  f'stroke-width="3" marker-end="url(#ar)"/>')
e(f'<rect x="{M}" y="{OY}" width="{W-2*M}" height="112" fill="#FFFFFF" '
  f'stroke="{RULE}" stroke-width="3"/>')
t(M + 26, OY + 46, "Answer context", 32, INK, "700", SANS)
t(M + 26, OY + 86, "one entry per memory, each with its verbatim quote, timestamp and source id",
  26, BODY, "400", MONO)
t(W - M - 26, OY + 68, "at or below B = 1000 tokens", 27, INK, "700", SANS, anchor="end")

e("</svg>")
p = Path(__file__).parent / "arch_v9.svg"
p.write_text("\n".join(o))
print(f"wrote {p} ({W}x{H})")
