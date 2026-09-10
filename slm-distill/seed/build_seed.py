"""Build a seeded PostgreSQL dump for the hosted demo.

Takes the 3,706 teacher-extracted memories and turns them into a realistic
memory-engine database: sessions, raw events, evidence chunks, canonical
memories, and — the part that matters — *version chains* where the same
canonical key was asserted more than once over time.

Those chains are what distinguish this from a vector-search toy. A memory that
was corrected keeps both versions, linked by a `supersedes` relation, each with
its own validity window and its own source quote.

Embeddings come from Azure text-embedding-3-small with dimensions=1024, which
matches the vector(1024) column the migration already declares.

  python seed/build_seed.py --out seed/seed.sql
"""

import argparse
import json
import re
import unicodedata
import uuid
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NS = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")  # fixed => reproducible ids
NAMESPACE = "demo"
EXTRACTOR_VERSION = "qwen3-1.7b-memex-lora@v1"

# our extraction types -> the kinds the Rust domain accepts
KIND = {"fact": "fact", "preference": "preference", "episode": "other", "task": "goal"}

MONTHS = {m: i + 1 for i, m in enumerate(
    ["january", "february", "march", "april", "may", "june", "july",
     "august", "september", "october", "november", "december"])}


def uid(*parts) -> str:
    return str(uuid.uuid5(NS, "|".join(str(p) for p in parts)))


def q(s) -> str:
    """SQL string literal."""
    if s is None:
        return "NULL"
    s = unicodedata.normalize("NFC", str(s)).replace("\x00", "")
    return "'" + s.replace("'", "''") + "'"


def norm(s: str) -> str:
    """Mirror domain::normalize_component exactly."""
    s = " ".join(str(s).split())
    return re.sub(r"^[^0-9a-zA-Z]+|[^0-9a-zA-Z]+$", "", s).lower()


def canonical_key(subject: str, predicate: str) -> str:
    return f"{norm(subject)}::{norm(predicate)}"


STOPW = {"a", "an", "the", "his", "her", "their", "some", "multiple", "several",
         "many", "few", "of", "to", "with", "and", "in", "on", "at", "for"}


def value_tokens(v: str) -> frozenset:
    return frozenset(w for w in re.findall(r"[a-z0-9]+", str(v).lower())
                     if w not in STOPW)


def same_value(a: str, b: str) -> bool:
    """True when two asserted values are the same claim worded differently.

    'dogs' / 'multiple dogs' / 'four dogs' are one fact restated, not a
    correction. Treating each restatement as a supersession produced 22-version
    chains that no reviewer would believe, so they are merged instead.
    """
    ta, tb = value_tokens(a), value_tokens(b)
    if not ta or not tb:
        return norm(a) == norm(b)
    if ta == tb or ta <= tb or tb <= ta:
        return True
    inter = len(ta & tb)
    return inter / len(ta | tb) >= 0.5


# Predicates that hold many values at once. A person enjoys chess AND movies;
# neither supersedes the other, so these become separate memories keyed by value.
LIST_PREDICATES = {"has", "enjoys", "likes", "loves", "owns", "plays", "wants",
                   "is interested in", "is passionate about", "values", "does",
                   "practices", "uses", "needs", "hates", "dislikes", "prefers"}
MAX_CHAIN = 4


def parse_dt(s: str, fallback_days: int) -> datetime:
    """LoCoMo dates look like '1:56 pm on 8 May, 2023'."""
    m = re.search(r"(\d{1,2}):(\d{2})\s*(am|pm).*?(\d{1,2})\s+([A-Za-z]+),?\s*(\d{4})",
                  str(s), re.I)
    if m:
        hh, mm, ap, day, mon, yr = m.groups()
        hh, mm, day, yr = int(hh), int(mm), int(day), int(yr)
        if ap.lower() == "pm" and hh != 12:
            hh += 12
        if ap.lower() == "am" and hh == 12:
            hh = 0
        mo = MONTHS.get(mon.lower())
        if mo:
            try:
                return datetime(yr, mo, day, hh, mm, tzinfo=timezone.utc)
            except ValueError:
                pass
    return datetime(2023, 1, 1, tzinfo=timezone.utc) + timedelta(days=fallback_days)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", default="data/silver.jsonl")
    ap.add_argument("--out", default="seed/seed.sql")
    ap.add_argument("--emb", default="seed/embeddings.jsonl",
                    help="cache of statement -> vector")
    args = ap.parse_args()

    windows = [json.loads(l) for l in (ROOT / args.data).read_text().splitlines()]
    print(f"windows: {len(windows)}")

    # ---------------------------------------------------------------- collect
    # one session per LoCoMo conversation-session, one raw_event per window,
    # one chunk per supporting turn.
    sessions, events, chunks = {}, [], []
    facts = []                      # every asserted memory, in time order
    for w in windows:
        conv, sess, off = w["id"].split("::")
        skey = f"{conv}::{sess}"
        when = parse_dt(w["session_date"], len(sessions))
        if skey not in sessions:
            sessions[skey] = {"id": uid("session", skey), "external_id": skey,
                              "created_at": when}
        sid = sessions[skey]["id"]

        body = "\n".join(f"{t['speaker']}: {t['text']}" for t in w["turns"])
        ev_id = uid("event", w["id"])
        # windows within a session are minutes apart, so ordering is stable
        occurred = when + timedelta(minutes=int(off))
        events.append({"id": ev_id, "session_id": sid, "content": body,
                       "occurred_at": occurred})

        by_dia = {}
        for i, t in enumerate(w["turns"]):
            cid = uid("chunk", w["id"], t["dia_id"])
            chunks.append({"id": cid, "raw_event_id": ev_id, "ordinal": i,
                           "content": f"{t['speaker']}: {t['text']}"})
            by_dia[t["dia_id"]] = cid

        for j, m in enumerate(w["target"]["memories"]):
            ck = canonical_key(m["subject"], m["predicate"])
            if not ck.strip(":"):
                continue
            src = [by_dia[d] for d in m.get("evidence", []) if d in by_dia]
            facts.append({
                "ck": ck, "subject": m["subject"], "predicate": m["predicate"],
                "value": m["object"], "statement": m["statement"],
                "kind": KIND.get(m["type"], "other"),
                "at": occurred, "sources": src, "seq": (occurred, w["id"], j),
            })

    facts.sort(key=lambda f: f["seq"])
    print(f"sessions {len(sessions)}  events {len(events)}  chunks {len(chunks)}  "
          f"asserted memories {len(facts)}")

    # ------------------------------------------------- build version chains
    groups = defaultdict(list)
    for f in facts:
        groups[f["ck"]].append(f)

    memories, versions, sources, relations = [], [], [], []
    chains = 0
    for ck, items in groups.items():
        # collapse restatements of the same value, keeping the earliest assertion
        distinct = []
        for f in items:
            hit = next((d for d in distinct if same_value(d["value"], f["value"])), None)
            if hit:
                hit["sources"] = list(dict.fromkeys(hit["sources"] + f["sources"]))
                continue
            distinct.append(dict(f))

        pred = norm(items[0]["predicate"]).replace("_", " ")
        # Only a *stative* claim can be superseded: "owns a Prius" stops being
        # true when the Prius is sold. An episode accumulates instead — visiting
        # Paris does not make visiting Rome false — so events are never chained,
        # and generic stative verbs that hold many values at once behave the same.
        episodic = items[0]["kind"] == "other"
        if len(distinct) > 2 and (episodic or pred in LIST_PREDICATES) \
                or (episodic and len(distinct) > 1):
            for f in distinct:
                sub = f"{ck}::{norm(f['value'])}"[:200]
                mid = uid("memory", sub)
                memories.append({"id": mid, "canonical_key": sub,
                                 "subject": f["subject"], "predicate": f["predicate"],
                                 "created_at": f["at"]})
                vid = uid("version", sub, 1)
                versions.append({
                    "id": vid, "memory_id": mid, "version": 1, "value": f["value"],
                    "normalized_value": norm(f["value"]), "statement": f["statement"],
                    "kind": f["kind"], "status": "active", "valid_from": f["at"],
                    "valid_to": None, "extractor_version": EXTRACTOR_VERSION})
                for cid in f["sources"]:
                    sources.append((vid, cid))
            continue

        # keep the most recent versions; a plausible correction history is short
        if len(distinct) > MAX_CHAIN:
            distinct = distinct[-MAX_CHAIN:]
        if len(distinct) > 1:
            chains += 1

        mem_id = uid("memory", ck)
        memories.append({"id": mem_id, "canonical_key": ck,
                         "subject": distinct[0]["subject"],
                         "predicate": distinct[0]["predicate"],
                         "created_at": distinct[0]["at"]})
        prev_vid = None
        for n, f in enumerate(distinct, start=1):
            vid = uid("version", ck, n)
            last = n == len(distinct)
            versions.append({
                "id": vid, "memory_id": mem_id, "version": n, "value": f["value"],
                "normalized_value": norm(f["value"]), "statement": f["statement"],
                "kind": f["kind"], "status": "active" if last else "superseded",
                "valid_from": f["at"],
                "valid_to": None if last else distinct[n]["at"],
                "extractor_version": EXTRACTOR_VERSION})
            for cid in f["sources"]:
                sources.append((vid, cid))
            if prev_vid:
                relations.append((uid("rel", ck, n), vid, prev_vid, "supersedes"))
            prev_vid = vid

    print(f"canonical memories {len(memories)}  versions {len(versions)}  "
          f"chains with >1 version {chains}  relations {len(relations)}")

    # -------------------------------------------------------- embeddings
    cache = {}
    ep = ROOT / args.emb
    if ep.exists():
        for line in ep.read_text().splitlines():
            r = json.loads(line)
            cache[r["t"]] = r["v"]
        print(f"embedding cache: {len(cache)}")
    missing = [v["statement"] for v in versions if v["statement"] not in cache]
    print(f"statements needing embedding: {len(set(missing))}")
    (ROOT / "seed").mkdir(exist_ok=True)
    (ROOT / "seed" / "to_embed.json").write_text(
        json.dumps(sorted(set(missing)), indent=0))

    if missing:
        print("run  python seed/embed.py  then re-run this script")
        return

    # --------------------------------------------------------------- emit
    out = [
        "-- Generated by seed/build_seed.py. Do not edit.",
        "BEGIN;",
        "TRUNCATE memory_relations, memory_version_sources, memory_versions,",
        "         memories, chunks, raw_events, sessions RESTART IDENTITY CASCADE;",
    ]
    for s in sessions.values():
        out.append(f"INSERT INTO sessions (id,namespace,external_id,created_at) VALUES "
                   f"({q(s['id'])},{q(NAMESPACE)},{q(s['external_id'])},"
                   f"{q(s['created_at'].isoformat())});")
    for e in events:
        out.append(f"INSERT INTO raw_events (id,session_id,role,content,occurred_at,"
                   f"processing_state) VALUES ({q(e['id'])},{q(e['session_id'])},"
                   f"'user',{q(e['content'])},{q(e['occurred_at'].isoformat())},"
                   f"'processed');")
    for c in chunks:
        out.append(f"INSERT INTO chunks (id,raw_event_id,ordinal,content) VALUES "
                   f"({q(c['id'])},{q(c['raw_event_id'])},{c['ordinal']},"
                   f"{q(c['content'])});")
    for m in memories:
        out.append(f"INSERT INTO memories (id,namespace,canonical_key,subject,predicate,"
                   f"created_at) VALUES ({q(m['id'])},{q(NAMESPACE)},"
                   f"{q(m['canonical_key'])},{q(m['subject'])},{q(m['predicate'])},"
                   f"{q(m['created_at'].isoformat())});")
    for v in versions:
        vec = cache[v["statement"]]
        emb = "'[" + ",".join(f"{x:.6f}" for x in vec) + "]'::vector"
        vt = q(v["valid_to"].isoformat()) if v["valid_to"] else "NULL"
        out.append(f"INSERT INTO memory_versions (id,memory_id,version,value,"
                   f"normalized_value,statement,kind,status,valid_from,valid_to,"
                   f"extractor_version,embedding) VALUES ({q(v['id'])},"
                   f"{q(v['memory_id'])},{v['version']},{q(v['value'])},"
                   f"{q(v['normalized_value'])},{q(v['statement'])},{q(v['kind'])},"
                   f"{q(v['status'])},{q(v['valid_from'].isoformat())},{vt},"
                   f"{q(v['extractor_version'])},{emb});")
    for vid, cid in sources:
        out.append(f"INSERT INTO memory_version_sources (memory_version_id,chunk_id) "
                   f"VALUES ({q(vid)},{q(cid)}) ON CONFLICT DO NOTHING;")
    for rid, frm, to, kind in relations:
        out.append(f"INSERT INTO memory_relations (id,from_version_id,to_version_id,"
                   f"relation_type) VALUES ({q(rid)},{q(frm)},{q(to)},{q(kind)});")
    out.append("COMMIT;")

    p = ROOT / args.out
    p.write_text("\n".join(out))
    print(f"\nwrote {p}  ({p.stat().st_size/1e6:.1f} MB, {len(out)} statements)")


if __name__ == "__main__":
    main()
