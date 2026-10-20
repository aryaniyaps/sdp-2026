#!/usr/bin/env python3
"""Deterministic memory-property suite: scripted conversations with checks that need no judge.

Each scenario is retained into its own namespace and checked against the engine's stored assertions and
its recall/reflect responses. The families test the behaviours the report claims: knowledge updates keep
history, relative dates are resolved, the model abstains without evidence, another person's news does
not overwrite the user's own facts, and credentials are never stored. Every recall source quote is also
checked against the retained text. Scenarios are generated from fixed templates and a seed; they are a
functional test of the engine, not an independent benchmark (LoCoMo is that).

    python3 benchmark/properties.py run [--run properties-v1] [--reader]
"""

from __future__ import annotations
import argparse
import datetime as dt
import json
import random
import re
import urllib.parse

import run as bench

CITIES = ["Pune", "Chennai", "Lisbon", "Toronto", "Nairobi", "Osaka", "Denver", "Krakow", "Hobart", "Recife",
          "Tallinn", "Accra"]
COMPANIES = ["Northwind", "Bluefin Labs", "Orchid Health", "Kestrel Freight", "Juniper Bank", "Solace Games",
             "Tidewater Energy", "Quarry Studio"]
CARS = ["Honda Civic", "Tesla Model 3", "Subaru Outback", "Mazda CX-5", "Toyota Corolla", "Kia Niro"]
PEOPLE = ["Maya Sen", "Tomas Ruiz", "Aiko Tanaka", "Lena Novak", "Kofi Mensah", "Priya Raman", "Owen Clarke",
          "Sara Haddad", "Iris Lund", "Mateo Silva", "Nadia Petrova", "Ravi Iyer"]
FRIENDS = ["Jo", "Sam", "Alex", "Rin", "Dev", "Mia"]
HOBBIES = ["rock climbing", "pottery", "chess", "birdwatching", "salsa dancing", "sourdough baking"]
# (past form, future form, noun for the question, keyword the stored assertion must mention)
EVENTS = [("had a dentist appointment", "have a dentist appointment", "dentist appointment", "dentist"),
          ("ran a half marathon", "run a half marathon", "half marathon", "marathon"),
          ("visited the science museum", "visit the science museum", "science museum visit", "museum"),
          ("took my driving test", "take my driving test", "driving test", "driving test"),
          ("adopted a kitten", "adopt a kitten", "kitten adoption", "kitten"),
          ("presented at the design review", "present at the design review", "design review talk", "design review")]
RELATIVE = [("yesterday", -1), ("tomorrow", 1), ("two days ago", -2), ("in three days", 3)]


def at(day, hour=18):
    return (dt.datetime(2025, 1, 6, tzinfo=dt.timezone.utc) + dt.timedelta(days=day, hours=hour)).isoformat()


def session(name, friend, lines, day):
    """A short two-person session in LoCoMo's 'Speaker: text' form, all turns at one timestamp."""
    turns = [f"{friend}: Hey {name.split()[0]}! How have you been?"]
    turns += [f"{name}: {line}" for line in lines]
    turns.append(f"{friend}: Thanks for telling me, that's good to know.")
    return {"date": at(day), "turns": turns}


def scenarios(seed):
    rng = random.Random(seed)
    out = []

    def person():
        return rng.choice(PEOPLE), rng.choice(FRIENDS)

    updates = [
        ("city", CITIES, "I live in {old} these days, it's been a good place for me.",
         "Big news: I moved from {old} to {new} last weekend, so I live in {new} now.",
         "Where does {first} live?"),
        ("employer", COMPANIES, "I work at {old} now and I really like the team there.",
         "I left {old} and started a new job at {new} this month.",
         "Where does {first} work?"),
        ("car", CARS, "I drive a {old}, it has been very reliable.",
         "I sold my {old} and bought a {new} last week.",
         "What car does {first} drive?"),
    ]
    for slot, pool, first_line, second_line, question in updates:
        for k in range(4):
            name, friend = person()
            old, new = rng.sample(pool, 2)
            day = 20 * len(out)
            fill = {"old": old, "new": new, "first": name.split()[0]}
            out.append({
                "id": f"update-{slot}-{k}", "family": "knowledge_update", "name": name,
                "sessions": [session(name, friend, [first_line.format(**fill)], day),
                             session(name, friend, [second_line.format(**fill)], day + 40)],
                "question": question.format(**fill), "asked": at(day + 41), "past": at(day + 10),
                "expect": new, "old": old,
            })
    for k in range(12):
        name, friend = person()
        past, future, noun, keyword = EVENTS[k % len(EVENTS)]
        relative, delta = RELATIVE[k % len(RELATIVE)]
        line = f"I will {future} {relative}." if delta > 0 else f"I {past} {relative}."
        day = 20 * len(out)
        event_day = (dt.datetime.fromisoformat(at(day)) + dt.timedelta(days=delta)).date().isoformat()
        out.append({
            "id": f"temporal-{relative.replace(' ', '_')}-{k}", "family": "relative_date", "name": name,
            "sessions": [session(name, friend, [line], day)],
            "question": f"When is or was {name.split()[0]}'s {noun}?", "asked": at(day + 5),
            "keyword": keyword, "expect_date": event_day, "relative": relative,
        })
    for k in range(6):
        name, friend = person()
        city = rng.choice(CITIES)
        day = 20 * len(out)
        out.append({
            "id": f"abstain-{k}", "family": "abstention", "name": name,
            "sessions": [session(name, friend, [f"I live in {city} and I love {rng.choice(HOBBIES)}."], day)],
            "question": f"What is {name.split()[0]}'s favourite film?", "control": f"Where does {name.split()[0]} live?",
            "asked": at(day + 2), "expect": city,
        })
    for k in range(6):
        name, friend = person()
        mine, theirs = rng.sample(CITIES, 2)
        relative = rng.choice(["cousin", "flatmate", "colleague"])
        other = rng.choice([p for p in PEOPLE if p != name]).split()[0]
        day = 20 * len(out)
        out.append({
            "id": f"attribution-{k}", "family": "attribution", "name": name,
            "sessions": [session(name, friend, [f"I live in {mine}, I've been here for years."], day),
                         session(name, friend, [f"My {relative} {other} just moved to {theirs} for a new job!"], day + 30)],
            "question": f"Where does {name.split()[0]} live?", "asked": at(day + 31), "expect": mine, "old": theirs,
        })
    for k in range(4):
        name, friend = person()
        hobby = rng.choice(HOBBIES)
        key = "sk-live-" + "".join(rng.choice("abcdefghijkmnpqrstuvwxyz23456789") for _ in range(24))
        day = 20 * len(out)
        out.append({
            "id": f"secret-{k}", "family": "credential", "name": name,
            "sessions": [session(name, friend, [f"My work API key is {key}, please keep it private.",
                                                f"Also, I've really got into {hobby} lately."], day)],
            "question": f"What hobby has {name.split()[0]} taken up?", "asked": at(day + 2),
            "secret": key, "expect": hobby,
        })
    return out


def retain(base, ns, scenario):
    for index, s in enumerate(scenario["sessions"]):
        bench.api(base, "/api/v2/retain", {
            "namespace": ns, "session_id": f"session_{index + 1}", "external_id": f"{scenario['id']}:session_{index + 1}",
            "events": [{"role": "user", "content": t, "occurred_at": s["date"], "metadata": {"turn": j}}
                       for j, t in enumerate(s["turns"])],
        })
        # Sessions are retained one at a time, as they would arrive in a live deployment.
        bench.wait_ready(base, ns, poll=3)


def mentions(text, value):
    return value.lower() in (text or "").lower()


def recall(base, ns, query, asked, **extra):
    return bench.api(base, "/api/v2/recall", {"namespace": ns, "query": query, "reference_date": asked,
                                              "max_tokens": bench.CONFIG["context_budget"],
                                              "top_k": bench.CONFIG["top_k"], **extra}, timeout=900)


def check(base, ns, scenario):
    graph = bench.api(base, f"/api/v2/graph?namespace={urllib.parse.quote(ns)}")
    assertions = [a for a in graph["assertions"] if a["kind"] != "inference"]
    active = [a for a in assertions if a["status"] in ("active", "contested")]
    family, checks, responses = scenario["family"], {}, {}
    if family == "knowledge_update":
        new, old = scenario["expect"], scenario["old"]
        checks["new_value_active"] = any(mentions(a["statement"], new) for a in active)
        checks["old_value_kept_as_history"] = any(
            mentions(a["statement"], old) and a["status"] == "superseded" and a.get("valid_to") for a in assertions)
        checks["old_value_not_current"] = not any(
            mentions(a["statement"], old) and not mentions(a["statement"], new) for a in active)
        now = responses["recall_now"] = recall(base, ns, scenario["question"], scenario["asked"])
        first = next((h for h in now["results"] if mentions(h["statement"], new) or mentions(h["statement"], old)), None)
        checks["recall_ranks_current_value_first"] = bool(first) and mentions(first["statement"], new)
        past = responses["recall_as_of"] = recall(base, ns, scenario["question"], scenario["asked"], as_of=scenario["past"])
        checks["as_of_recall_returns_old_value"] = any(
            mentions(h["statement"], old) and not mentions(h["statement"], new) for h in past["results"])
    elif family == "relative_date":
        hits = [a for a in assertions if mentions(a["statement"], scenario["keyword"])]
        responses["matching_assertions"] = hits
        checks["event_extracted"] = bool(hits)
        checks["relative_date_resolved"] = any(
            (a.get("event_at") or "")[:10] == scenario["expect_date"] for a in hits)
    elif family == "abstention":
        unknown = responses["reflect_unknown"] = bench.api(base, "/api/v2/reflect", {
            "namespace": ns, "query": scenario["question"], "reference_date": scenario["asked"]}, timeout=900)
        known = responses["reflect_control"] = bench.api(base, "/api/v2/reflect", {
            "namespace": ns, "query": scenario["control"], "reference_date": scenario["asked"]}, timeout=900)
        checks["abstains_without_evidence"] = bool(unknown.get("insufficient_evidence"))
        checks["answers_with_evidence"] = not known.get("insufficient_evidence") and mentions(known.get("answer"), scenario["expect"])
    elif family == "attribution":
        mine, theirs = scenario["expect"], scenario["old"]
        first = scenario["name"].split()[0]
        checks["own_city_still_active"] = any(mentions(a["statement"], mine) for a in active)
        checks["own_city_not_superseded"] = not any(
            mentions(a["statement"], mine) and a["status"] == "superseded" for a in assertions)
        checks["relative_move_not_attributed_to_user"] = not any(
            mentions(a["statement"], theirs) and mentions(a["statement"], first) and not re.search(
                r"cousin|flatmate|colleague", a["statement"], re.I) for a in active)
    elif family == "credential":
        checks["secret_never_stored"] = not any(scenario["secret"] in json.dumps(a) for a in graph["assertions"])
        checks["ordinary_fact_kept"] = any(mentions(a["statement"], scenario["expect"]) for a in active)
    # Provenance: every source quote that recall returns occurs verbatim in a retained turn.
    texts = [t for s in scenario["sessions"] for t in s["turns"]]
    probe = responses.get("recall_now") or recall(base, ns, scenario["question"], scenario["asked"])
    quotes = [src["quote"] for h in probe["results"] for src in h["sources"]]
    checks["source_quotes_verbatim"] = all(any(q and q in t for t in texts) for q in quotes)
    return checks, {"assertions": graph["assertions"], **responses}


def reader_comparison(base, ns, scenario):
    """The same question answered by the benchmark reader from three contexts, scored by string match."""
    if scenario["family"] not in ("knowledge_update", "relative_date", "attribution"):
        return {}
    result = {}
    transcript = "\n\n".join(f"[session {i + 1} | {s['date']}]\n" + "\n".join(s["turns"])
                             for i, s in enumerate(scenario["sessions"]))
    contexts = {"full_context": transcript}
    for condition in ("raw_hybrid", "enhanced"):
        contexts[condition] = recall(base, ns, scenario["question"], scenario["asked"],
                                     **bench.CONDITIONS[condition]["recall"])["context"]
    for condition, context in contexts.items():
        prompt = bench.reader_prompt({"question": scenario["question"]}, context)
        _, hypothesis, _ = bench.retry(lambda: bench.read_answer(prompt), "reader")
        if scenario["family"] == "relative_date":
            day = dt.date.fromisoformat(scenario["expect_date"])
            forms = {day.isoformat(), f"{day.day} {day:%B}", f"{day:%B} {day.day}"}
            correct = any(f.lower() in hypothesis.lower() for f in forms)
        else:
            correct = mentions(hypothesis, scenario["expect"]) and not mentions(hypothesis, scenario["old"])
        result[condition] = {"hypothesis": hypothesis, "correct": correct}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("stage", choices=["run", "report"])
    parser.add_argument("--run", default="properties-dev")
    parser.add_argument("--base", default=f"http://{bench.CONFIG['engine']['bind']}")
    parser.add_argument("--reader", action="store_true", help="also compare reader answers across contexts")
    args = parser.parse_args()
    folder = bench.BENCH / "runs" / args.run
    suite = scenarios(bench.CONFIG["seed"])
    if args.stage == "run":
        bench.check_service(args.base)
        folder.mkdir(parents=True, exist_ok=True)

        def one(scenario):
            path = folder / "scenarios" / f"{scenario['id']}.json"
            if path.exists():
                return
            ns = f"prop:{args.run}:{scenario['id']}"
            retain(args.base, ns, scenario)
            checks, evidence = check(args.base, ns, scenario)
            record = {"scenario": scenario, "namespace": ns, "checks": checks, "evidence": evidence}
            if args.reader:
                record["reader"] = reader_comparison(args.base, ns, scenario)
            bench.atomic_json(path, record)
            bench.log(stage="property", id=scenario["id"], passed=all(checks.values()), checks=checks)

        bench.parallel(one, suite, bench.CONFIG["engine"]["worker_concurrency"])
    records = [bench.read_json(folder / "scenarios" / f"{s['id']}.json") for s in suite
               if (folder / "scenarios" / f"{s['id']}.json").exists()]
    summary = {"scenarios": len(records), "families": {}, "checks": {}, "reader": {}}
    for r in records:
        family = summary["families"].setdefault(r["scenario"]["family"], {"n": 0, "all_checks_passed": 0})
        family["n"] += 1
        family["all_checks_passed"] += all(r["checks"].values())
        for name, ok in r["checks"].items():
            entry = summary["checks"].setdefault(f"{r['scenario']['family']}.{name}", {"n": 0, "passed": 0})
            entry["n"] += 1
            entry["passed"] += bool(ok)
        for condition, value in r.get("reader", {}).items():
            entry = summary["reader"].setdefault(r["scenario"]["family"], {}).setdefault(condition, {"n": 0, "correct": 0})
            entry["n"] += 1
            entry["correct"] += value["correct"]
    bench.atomic_json(folder / "summary.json", summary)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
