"""Generate the system architecture diagram (slide 7).

Module-level, not function-level: every box is a Rust module that exists in the
crate. Database tables are drawn with their actual columns so the panel can see
what is stored, not just that "a database" exists.

    python diagrams/arch.py && rsvg-convert -w 3200 diagrams/arch.svg -o diagrams/arch.png
"""

from pathlib import Path

W, H = 3200, 1600

INK = "#1B2536"       # primary line / text
MUTED = "#5B6779"     # secondary text
BAND_BG = "#F5F7FA"
BAND_LINE = "#C3CAD6"
BOX_LINE = "#2F3B52"
ACC = "#1F6FEB"       # accent: the SLM subsystem
ACC_BG = "#EAF2FE"
HDR = "#2F3B52"       # table header bar

SANS = "DejaVu Sans, Helvetica, Arial, sans-serif"
MONO = "DejaVu Sans Mono, Menlo, Consolas, monospace"

out: list[str] = []
def e(s: str) -> None:
    out.append(s)


def esc(t: str) -> str:
    return t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def band(x, y, w, h, num, title, sub=""):
    e(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="16" '
      f'fill="{BAND_BG}" stroke="{BAND_LINE}" stroke-width="2"/>')
    e(f'<circle cx="{x+44}" cy="{y+46}" r="23" fill="{INK}"/>')
    e(f'<text x="{x+44}" y="{y+57}" font-family="{SANS}" font-size="28" font-weight="700" '
      f'fill="#fff" text-anchor="middle">{num}</text>')
    e(f'<text x="{x+84}" y="{y+57}" font-family="{SANS}" font-size="34" font-weight="700" '
      f'fill="{INK}" letter-spacing="1.5">{esc(title)}</text>')
    if sub:
        # 34px bold + 1.5 letter-spacing runs ~23.5px/char; +40 for the gap
        tw = 84 + int(len(title) * 23.5) + 40
        e(f'<text x="{x+tw}" y="{y+57}" font-family="{SANS}" font-size="27" '
          f'fill="{MUTED}">{esc(sub)}</text>')


def module(x, y, w, h, name, role, bullets, accent=False):
    line = ACC if accent else BOX_LINE
    fill = ACC_BG if accent else "#FFFFFF"
    e(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{fill}" '
      f'stroke="{line}" stroke-width="{4 if accent else 2.5}"/>')
    e(f'<text x="{x+24}" y="{y+52}" font-family="{MONO}" font-size="38" font-weight="700" '
      f'fill="{ACC if accent else INK}">{esc(name)}</text>')
    if accent:
        bx = x + w - 108
        e(f'<rect x="{bx}" y="{y+22}" width="86" height="36" rx="18" fill="{ACC}"/>')
        e(f'<text x="{bx+43}" y="{y+48}" font-family="{SANS}" font-size="24" font-weight="700" '
          f'fill="#fff" text-anchor="middle">SLM</text>')
    e(f'<text x="{x+24}" y="{y+92}" font-family="{SANS}" font-size="27" fill="{MUTED}">'
      f'{esc(role)}</text>')
    e(f'<line x1="{x+24}" y1="{y+112}" x2="{x+w-24}" y2="{y+112}" stroke="{BAND_LINE}" '
      f'stroke-width="1.5"/>')
    ty = y + 152
    for b in bullets:
        e(f'<circle cx="{x+32}" cy="{ty-9}" r="4.5" fill="{ACC if accent else MUTED}"/>')
        e(f'<text x="{x+50}" y="{ty}" font-family="{SANS}" font-size="26" fill="{INK}">'
          f'{esc(b)}</text>')
        ty += 40


def table(x, y, w, h, name, cols, note=""):
    e(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="#FFFFFF" '
      f'stroke="{BOX_LINE}" stroke-width="2.5"/>')
    e(f'<path d="M{x} {y+52} L{x} {y+10} Q{x} {y} {x+10} {y} L{x+w-10} {y} '
      f'Q{x+w} {y} {x+w} {y+10} L{x+w} {y+52} Z" fill="{HDR}"/>')
    e(f'<text x="{x+18}" y="{y+37}" font-family="{MONO}" font-size="29" font-weight="700" '
      f'fill="#fff">{esc(name)}</text>')
    ty = y + 92
    for c, kind in cols:
        col = ACC if kind == "idx" else (MUTED if kind == "fk" else INK)
        wt = "700" if kind == "pk" else "400"
        e(f'<text x="{x+18}" y="{ty}" font-family="{MONO}" font-size="24" font-weight="{wt}" '
          f'fill="{col}">{esc(c)}</text>')
        ty += 34
    if note:
        e(f'<text x="{x+18}" y="{y+h-16}" font-family="{SANS}" font-size="22" '
          f'font-style="italic" fill="{MUTED}">{esc(note)}</text>')


def arrow(x1, y1, x2, y2, label="", accent=False):
    c = ACC if accent else BOX_LINE
    m = "urlarrowA" if accent else "urlarrow"
    e(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{c}" stroke-width="3.5" '
      f'marker-end="url(#{"arrA" if accent else "arr"})"/>')
    if label:
        mx, my = (x1 + x2) / 2, (y1 + y2) / 2
        e(f'<text x="{mx+14}" y="{my-8}" font-family="{SANS}" font-size="24" '
          f'fill="{MUTED}">{esc(label)}</text>')


e(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">')
e('<defs>')
for mid, col in (("arr", BOX_LINE), ("arrA", ACC)):
    e(f'<marker id="{mid}" viewBox="0 0 12 12" refX="10" refY="6" markerWidth="8" '
      f'markerHeight="8" orient="auto"><path d="M0 0 L12 6 L0 12 z" fill="{col}"/></marker>')
e('</defs>')
e(f'<rect width="{W}" height="{H}" fill="#FFFFFF"/>')

# ---------------------------------------------------------------- 1 write path
band(40, 40, 3120, 430, "1", "WRITE PATH",
     "crate modules that turn interaction text into versioned, cited memory")
MW, MH, MY = 578, 310, 126
xs = [66, 688, 1310, 1932, 2554]
module(xs[0], MY, MW, MH, "api", "axum HTTP boundary", [
    "POST /sessions/:id/events", "validate, assign request_id", "hand off to ingest worker"])
module(xs[1], MY, MW, MH, "ingest", "normalise + chunk", [
    "speaker, event_time, scope", "split into evidence chunks", "write raw_events + chunks"])
module(xs[2], MY, MW, MH, "extract", "typed memory extraction", [
    "Qwen3-4B student, 4-bit local", "JSON-schema constrained decode",
    "confidence router -> teacher"], accent=True)
module(xs[3], MY, MW, MH, "identity", "canonicalise + dedupe", [
    "entity_key = subject::predicate", "cosine + hash near-dup match",
    "resolve to existing memory"])
module(xs[4], MY, MW, MH, "versioning", "temporal version control", [
    "supersede / extend / derive", "close valid_to, open new row",
    "both versions retained"])
for i in range(4):
    arrow(xs[i] + MW + 4, MY + MH / 2, xs[i + 1] - 6, MY + MH / 2)

# --------------------------------------------------------------- 2 persistence
band(40, 492, 3120, 576, "2", "PERSISTENCE",
     "one PostgreSQL 17 instance - tables and indexes as deployed")
TY, TH = 572, 468
table(66, TY, 519, TH, "raw_events", [
    ("id           uuid", "pk"), ("session_id   uuid", ""), ("speaker      text", ""),
    ("body         text", ""), ("event_time   timestamptz", ""),
    ("ingested_at  timestamptz", "")], "immutable - never updated")
table(625, TY, 519, TH, "chunks", [
    ("id           uuid", "pk"), ("raw_event_id ->raw_events", "fk"),
    ("dia_id       text", ""), ("text         text", ""),
    ("token_count  int", "")], "the quoted evidence")
table(1184, TY, 519, TH, "memories", [
    ("id            uuid", "pk"), ("entity_key    text UNIQUE", ""),
    ("type          fact|preference", ""), ("              |episode|task", ""),
    ("subject       text", ""), ("predicate     text", ""),
    ("current_ver   ->versions", "fk")], "one row per canonical claim")
table(1743, TY, 830, TH, "memory_versions", [
    ("id                uuid", "pk"), ("memory_id         ->memories", "fk"),
    ("object, statement text", ""), ("valid_from        timestamptz", ""),
    ("valid_to          timestamptz NULL=live", ""),
    ("status            active | superseded", ""),
    ("confidence        real", ""),
    ("extractor_version text  <- pins the SLM", ""),
    ("embedding         vector(1024)   HNSW", "idx"),
    ("fts               tsvector       GIN", "idx")], "history is append-only")
table(2613, TY, 519, TH, "relations", [
    ("id           uuid", "pk"), ("src_version  ->versions", "fk"),
    ("dst_version  ->versions", "fk"), ("kind         supersedes", ""),
    ("             | extends", ""), ("             | derives", ""),
    ("created_at   timestamptz", "")], "the audit edges")

arrow(977, MY + MH + 4, 977, TY - 8, "evidence")
arrow(2843, MY + MH + 4, 2843, TY - 8, "versions + edges")

# ----------------------------------------------------------------- 3 read path
band(40, 1092, 2470, 462, "3", "READ PATH", "token-bounded, cited context")
RW, RH, RY = 571, 340, 1182
rxs = [66, 681, 1296, 1911]
module(rxs[0], RY, RW, RH, "api", "GET /context", [
    "query, session, time window", "returns quotes + provenance"])
module(rxs[1], RY, RW, RH, "search", "hybrid candidate fetch", [
    "FTS top-20 || pgvector top-20", "hard filter: valid_to IS NULL", "runs both in parallel"])
module(rxs[2], RY, RW, RH, "fusion", "reciprocal rank fusion", [
    "RRF over the two lists", "recency + salience weights", "drop superseded versions"])
module(rxs[3], RY, RW, RH, "pack", "token-budget assembly", [
    "greedy pack to <= 1000 tok", "attach chunk quotes", "attach timestamps + ids"])
for i in range(3):
    arrow(rxs[i] + RW + 4, RY + RH / 2, rxs[i + 1] - 6, RY + RH / 2)
arrow(966, TY + TH + 4, 966, RY - 8, "candidates")

# ------------------------------------------------------------- 4 cross-cutting
band(2550, 1092, 610, 462, "4", "CROSS-CUTTING")
cx, cw = 2576, 558
for i, (nm, txt) in enumerate([
        ("embed", "qwen3-embedding 0.6b, 1024-d"),
        ("maintain", "consolidate, decay, re-embed"),
        ("observe", "request_id traces, p50/p95")]):
    by = 1160 + i * 128
    e(f'<rect x="{cx}" y="{by}" width="{cw}" height="118" rx="10" fill="#FFFFFF" '
      f'stroke="{BOX_LINE}" stroke-width="2.5"/>')
    e(f'<text x="{cx+20}" y="{by+45}" font-family="{MONO}" font-size="30" font-weight="700" '
      f'fill="{INK}">{esc(nm)}</text>')
    e(f'<text x="{cx+20}" y="{by+86}" font-family="{SANS}" font-size="24" fill="{MUTED}">'
      f'{esc(txt)}</text>')

e('</svg>')

p = Path(__file__).parent / "arch.svg"
p.write_text("\n".join(out))
print(f"wrote {p} ({W}x{H})")
