"""Generate the deployment diagram (slide 9).

The point this makes that the old diagram could not: the teacher model is used
ONCE, offline, and never appears on the serving path. That is what lets the
project keep its "runs locally, no cloud dependency" claim while still using a
hosted teacher for distillation.

    python diagrams/deploy.py && rsvg-convert -w 2800 -h 1350 diagrams/deploy.svg \
        -o diagrams/deploy.png
"""

from pathlib import Path

W, H = 2800, 1350

INK = "#1B2536"
MUTED = "#5B6779"
LINE = "#C3CAD6"
BOX = "#2F3B52"
ACC = "#1F6FEB"
ACC_BG = "#EAF2FE"
OFF_BG = "#FAFAFB"
RUN_BG = "#F5F7FA"
WARM = "#B3541E"

SANS = "DejaVu Sans, Helvetica, Arial, sans-serif"
MONO = "DejaVu Sans Mono, Menlo, Consolas, monospace"

out: list[str] = []
def e(s): out.append(s)
def esc(t): return t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def box(x, y, w, h, title, lines, mono_title=True, accent=False, warm=False):
    col = WARM if warm else (ACC if accent else BOX)
    fill = "#FFFFFF" if not accent else ACC_BG
    e(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="10" fill="{fill}" '
      f'stroke="{col}" stroke-width="{3.5 if accent else 2.5}"/>')
    e(f'<text x="{x+20}" y="{y+44}" font-family="{MONO if mono_title else SANS}" '
      f'font-size="30" font-weight="700" fill="{col}">{esc(title)}</text>')
    ty = y + 84
    for ln in lines:
        e(f'<text x="{x+20}" y="{ty}" font-family="{SANS}" font-size="24" fill="{INK}">'
          f'{esc(ln)}</text>')
        ty += 33


def cylinder(x, y, w, h, title, lines):
    ry = 20
    e(f'<path d="M{x} {y+ry} a{w/2} {ry} 0 0 1 {w} 0 v{h-2*ry} a{w/2} {ry} 0 0 1 {-w} 0 z" '
      f'fill="#FFFFFF" stroke="{BOX}" stroke-width="2.5"/>')
    e(f'<path d="M{x} {y+ry} a{w/2} {ry} 0 0 0 {w} 0" fill="none" stroke="{BOX}" '
      f'stroke-width="2.5"/>')
    e(f'<text x="{x+20}" y="{y+72}" font-family="{MONO}" font-size="28" font-weight="700" '
      f'fill="{INK}">{esc(title)}</text>')
    ty = y + 110
    for ln in lines:
        e(f'<text x="{x+20}" y="{ty}" font-family="{SANS}" font-size="23" fill="{MUTED}">'
          f'{esc(ln)}</text>')
        ty += 31


def arrow(x1, y1, x2, y2, col=BOX, dash=False):
    d = ' stroke-dasharray="10 8"' if dash else ""
    e(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{col}" stroke-width="3.5"'
      f'{d} marker-end="url(#a{"W" if col==WARM else ""})"/>')


e(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">')
e('<defs>')
for mid, col in (("a", BOX), ("aW", WARM)):
    e(f'<marker id="{mid}" viewBox="0 0 12 12" refX="10" refY="6" markerWidth="8" '
      f'markerHeight="8" orient="auto"><path d="M0 0 L12 6 L0 12 z" fill="{col}"/></marker>')
e('</defs>')
e(f'<rect width="{W}" height="{H}" fill="#FFFFFF"/>')

# ============================================================ A. offline, once
AX, AW = 40, 760
e(f'<rect x="{AX}" y="40" width="{AW}" height="1270" rx="16" fill="{OFF_BG}" '
  f'stroke="{WARM}" stroke-width="2.5" stroke-dasharray="12 8"/>')
e(f'<text x="{AX+26}" y="{84}" font-family="{SANS}" font-size="30" font-weight="700" '
  f'fill="{WARM}" letter-spacing="1.2">A. OFFLINE - ONCE</text>')
e(f'<text x="{AX+26}" y="{120}" font-family="{SANS}" font-size="23" fill="{MUTED}">'
  f'never on the serving path</text>')

box(AX + 26, 150, AW - 52, 200, "teacher", [
    "Azure gpt-5.6-sol (hosted)", "extracts typed memories from",
    "LoCoMo session windows"], warm=True)
box(AX + 26, 386, AW - 52, 200, "silver corpus", [
    "784 windows -> 3,706 memories", "642 train / 142 held-out val",
    "schema-validated, 0 failures"])
box(AX + 26, 622, AW - 52, 232, "distill", [
    "QLoRA on RTX 5070 Ti, 16 GB", "Qwen3-1.7B student, 4-bit NF4",
    "2 epochs, ~15 min", "loss masked to the JSON only"])
box(AX + 26, 890, AW - 52, 200, "adapter", [
    "LoRA weights + tokenizer", "stamped as extractor_version",
    "the only artefact that ships"], accent=True)

for y1, y2 in ((350, 380), (586, 616), (854, 884)):
    arrow(AX + AW / 2, y1, AX + AW / 2, y2, WARM)

e(f'<text x="{AX+26}" y="{1160}" font-family="{SANS}" font-size="24" font-weight="700" '
  f'fill="{WARM}">Cost: ~Rs 80 of credit, one run.</text>')
e(f'<text x="{AX+26}" y="{1196}" font-family="{SANS}" font-size="23" fill="{MUTED}">'
  f'Re-run only when the schema or</text>')
e(f'<text x="{AX+26}" y="{1228}" font-family="{SANS}" font-size="23" fill="{MUTED}">'
  f'the student model changes.</text>')

# hand-off
arrow(AX + AW + 6, 995, AX + AW + 128, 995, WARM)
e(f'<text x="{AX+AW+70}" y="{966}" font-family="{SANS}" font-size="22" fill="{WARM}" '
  f'text-anchor="middle">ships once</text>')

# ==================================================== B. local runtime, always
BX, BW = 940, 1820
e(f'<rect x="{BX}" y="40" width="{BW}" height="1270" rx="16" fill="{RUN_BG}" '
  f'stroke="{BOX}" stroke-width="3"/>')
e(f'<text x="{BX+30}" y="{84}" font-family="{SANS}" font-size="30" font-weight="700" '
  f'fill="{INK}" letter-spacing="1.2">B. LOCAL RUNTIME - ALWAYS</text>')
e(f'<text x="{BX+30}" y="{120}" font-family="{SANS}" font-size="23" fill="{MUTED}">'
  f'one workstation or lab server - no network egress at query time</text>')

# client + loopback
box(BX + 30, 156, 380, 170, "AI agent", [
    "same machine or", "trusted LAN"], mono_title=False)
box(BX + 30, 366, 380, 170, "127.0.0.1", [
    "loopback bind;", "DB never exposed"])
arrow(BX + 220, 330, BX + 220, 360)

# docker compose group
DX, DW = BX + 452, 1338
e(f'<rect x="{DX}" y="156" width="{DW}" height="656" rx="12" fill="#FFFFFF" '
  f'stroke="{LINE}" stroke-width="2.5" stroke-dasharray="8 6"/>')
e(f'<text x="{DX+22}" y="{196}" font-family="{SANS}" font-size="25" font-weight="700" '
  f'fill="{MUTED}" letter-spacing="1.2">DOCKER COMPOSE</text>')

box(DX + 26, 222, 1266, 150, "rust api", [
    "axum: store / search / update / delete    |    request_id on every call"])
box(DX + 26, 402, 405, 178, "worker", [
    "extract, embed,", "consolidate, repair"])
box(DX + 451, 402, 415, 178, "student slm", [
    "Qwen3-1.7B 4-bit", "~1.2 GB VRAM"], accent=True)
box(DX + 886, 402, 406, 178, "embed", [
    "qwen3-embedding 0.6b", "1,024-d vectors"])

cylinder(DX + 26, 620, 1266, 172, "postgresql 17 + pgvector", [
    "raw_events - chunks - memories - memory_versions - relations - traces",
    "GIN on fts    |    HNSW on embedding    |    single write transaction"])

# L-routed so the client reaches the API, not the model
e(f'<path d="M{BX+412} 451 H1375 V297 H{DX+16}" fill="none" stroke="{BOX}" '
  f'stroke-width="3.5" marker-end="url(#a)"/>')
arrow(DX + 228, 372, DX + 228, 398)
arrow(DX + 658, 372, DX + 658, 398)
arrow(DX + 1089, 372, DX + 1089, 398)
arrow(DX + 228, 580, DX + 228, 616)
arrow(DX + 1089, 580, DX + 1089, 616)

# host storage
e(f'<rect x="{DX}" y="890" width="{DW}" height="380" rx="12" fill="#FFFFFF" '
  f'stroke="{BOX}" stroke-width="2.5"/>')
e(f'<text x="{DX+22}" y="{932}" font-family="{SANS}" font-size="25" font-weight="700" '
  f'fill="{INK}" letter-spacing="1.2">HOST STORAGE BOUNDARY</text>')
cylinder(DX + 26, 962, 620, 160, "local volume", [
    "PostgreSQL data directory", "optional full-disk encryption"])
cylinder(DX + 672, 962, 620, 160, "backup", [
    "scheduled versioned snapshot", "offline / removable media"])
arrow(DX + 336, 812, DX + 336, 958)
arrow(DX + 646, 1042, DX + 668, 1042)

e(f'<text x="{DX+26}" y="{1188}" font-family="{SANS}" font-size="24" fill="{MUTED}">'
  f'Memory data never leaves this host. Only the API is reachable, and only from</text>')
e(f'<text x="{DX+26}" y="{1222}" font-family="{SANS}" font-size="24" fill="{MUTED}">'
  f'loopback or a trusted private address.</text>')

e('</svg>')

p = Path(__file__).parent / "deploy.svg"
p.write_text("\n".join(out))
print(f"wrote {p} ({W}x{H})")
