"""Coverage-focused synthetic episodes: long two-person conversations labelled with EVERY durable fact.

The general corpus (expand_general_corpus.py) asked for 3-6 short events with "normally 2-3 useful claims",
and the student learned to extract sparingly: on the LoCoMo dev conversations 77 of 126 wrong answers had
evidence that no stored assertion cited (benchmark/iterations.json, v5). These episodes are catch-up
conversations between two named people, 10-16 turns, whose labels cover every durable fact about either
speaker and named third parties. No benchmark text is used. Families are split before generation.

    PI_TEACHER_PROVIDER=claude-bridge PI_TEACHER_MODEL=claude-sonnet-5-5 \\
    PI_TEACHER_EXTENSION=~/.pi/agent/npm/node_modules/pi-claude-bridge/src/index.ts \\
    python expand_coverage_corpus.py --batches 160 --workers 6
"""
import argparse, hashlib, json, random
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import prompt
from expand_corpus import auxiliaries
from pi_teacher import PiTeacher
from research_corpus import validate_case
from teacher import TeacherError

ROOT = Path(__file__).parent / 'data/research-v4'
DOMAINS = ['family news and relatives', 'friendships and social plans', 'pets and their names', 'hobbies, crafts and art',
           'sports, fitness and outdoor trips', 'music, books and films', 'work, jobs and colleagues', 'study, courses and exams',
           'moving house and neighbourhoods', 'travel and holidays', 'food, cooking and restaurants', 'volunteering and community',
           'health appointments without medical advice', 'shopping, cars and possessions', 'celebrations and events attended',
           'personal goals and future plans']
SETTINGS = ['two old friends catching up after a few weeks', 'siblings chatting on a weekend call',
            'two colleagues talking at lunch', 'neighbours chatting about their week',
            'two friends planning to meet up', 'a long-distance friendship check-in']
BEHAVIOURS = ['relative dates (yesterday, last weekend, two days ago, next month) that must be resolved against the message time',
              'one speaker reports a change that replaces a fact in the existing facts',
              'a relative or friend of a speaker has news that must be attributed to that person, not the speaker',
              'plans and wishes that must stay separate from completed events',
              'named pets, children and partners whose names must be kept',
              'opinions and feelings about specific activities, books, places or events']


def one(teacher, index):
    path = ROOT / 'expanded' / f'batch-{index:03d}.json'
    if path.exists():
        return json.loads(path.read_text())
    domain, setting = DOMAINS[index % 16], SETTINGS[(index // 16) % len(SETTINGS)]
    behaviour = BEHAVIOURS[(index + index // 16) % len(BEHAVIOURS)]
    family = f'coverage-family-{index:03d}'
    # Ten families per domain: 8 train, 2 validation, assigned before generation.
    group = list(range(10)); random.Random(470 + index % 16).shuffle(group)
    split = 'val' if group.index((index // 16) % 10) < 2 else 'train'
    task = f'''Create four different synthetic memory extraction episodes. Each is a casual conversation: {setting}, mainly about {domain}. Also include: {behaviour}. Scenario family ID: {family}.
Each episode is ONE session of 10-16 short turns between two people with distinct fictional full names (vary cultures, ages and settings, no stereotypes, never reuse a name across cases). Every event has role "user" and content "First Last: message" or "First: message" (the speaker's name, a colon, then what they say), and all events share one RFC3339 UTC occurred_at (vary the date across cases, 2022-2026). Total content 1400-2200 characters. Write natural small talk with many concrete personal details: names of family members, pets, places, dates, activities, objects, plans, opinions.
Labels must be EXHAUSTIVE: one claim for every durable fact about either speaker or a named third party that a friend would remember later (events they did or will do, people and pets and their names, possessions, preferences, opinions about specific things, jobs, places, goals, feelings about an event). Typically 6-12 claims per episode; greetings, filler and small reactions are not claims. Each claim's subject is the person the fact is about. Statements name the person and resolve relative dates against occurred_at into absolute dates in the statement or value ("on 2023-05-07", "in June 2024"); event_at is null unless an exact clock time is given; never copy the message time as the event time. Each quote is an exact complete sentence or line of the cited event, including its speaker prefix only if you copy the full line.
For a correction, supply existing facts (real UUID id, subject, predicate, value, statement) and output only the changed value with correction=true. Existing facts are context, not new claims.
Return one JSON object: {{"cases":[{{"events":[{{"role":"user","content":"...","occurred_at":"RFC3339 UTC","metadata":{{}}}}],"existing":[],"claims":[canonical claims],"observations":[{{"subject_index":0,"support_indices":[0,1],"statement":"supported synthesis","predicate":"attribute","value":"brief value","confidence":0.85,"explanation":"support reason"}}],"reflection":{{"question":"follow-up question","answer":"answer only from NEW claims","citation_indices":[0],"insufficient_evidence":false}}}}]}}.
At least two cases need one observation combining two compatible claims about one subject, without invented causality. One reflection should be unanswerable from the claims (explicit insufficient-evidence answer, empty citation_indices, insufficient_evidence=true).
WORKER CONTRACT:\n{prompt.EXTRACT_TEMPLATE.split('Existing facts: {{EXISTING}}')[0]}'''
    system = 'Generate accurate synthetic supervised data for general-purpose memory. Follow the worker contract and verify every target against its supplied evidence.'
    try:
        value = teacher.complete_json(system, task, tag=family)
    except TeacherError as e:
        if 'not JSON' not in str(e):
            raise
        value = teacher.complete_json(system, task + '\nFormatting retry: return valid JSON with correctly escaped strings.', tag=family + '-json-retry')
    if len(value.get('cases', [])) != 4:
        raise ValueError('expected four cases')
    kept, errors = [], []
    for i, case in enumerate(value['cases']):
        tid = f'coverage-{index:03d}-{i}'
        try:
            row = validate_case(case, tid)
            row['meta']['source'] = 'coverage-synthetic-v4'
            coverage = {'domain': domain, 'behaviour': behaviour, 'setting': setting, 'family': family}
            for r in [row, *auxiliaries(case, row)]:
                r['coverage'] = coverage
                kept.append(r)
        except Exception as e:
            errors.append({'case': i, 'error': str(e)})
    result = {'index': index, 'split': split, 'coverage': {'domain': domain, 'behaviour': behaviour, 'setting': setting, 'family': family},
              'rows': kept, 'errors': errors, 'raw_cases': value['cases'], 'teacher': {'provider': teacher.provider, 'model': teacher.model},
              'contract_sha256': hashlib.sha256(prompt.EXTRACT_TEMPLATE.encode()).hexdigest(),
              'system_sha256': hashlib.sha256(prompt.SYSTEM.encode()).hexdigest()}
    path.write_text(json.dumps(result, ensure_ascii=False))
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--batches', type=int, default=160)
    ap.add_argument('--workers', type=int, default=6)
    ap.add_argument('--start', type=int, default=0)
    args = ap.parse_args()
    (ROOT / 'expanded').mkdir(parents=True, exist_ok=True)
    teacher = PiTeacher(ROOT / 'teacher-cache', ROOT / 'teacher-usage.jsonl')
    with ThreadPoolExecutor(args.workers) as pool:
        jobs = {pool.submit(one, teacher, i): i for i in range(args.start, args.batches)}
        for f in as_completed(jobs):
            try:
                r = f.result()
                claims = sum(len(json.loads(x['target'])['claims']) for x in r['rows'] if x['task'] == 'extract')
                print(r['index'], r['split'], len(r['rows']), 'rows', claims, 'claims', len(r['errors']), 'invalid cases', flush=True)
            except Exception as e:
                print('FAILED', jobs[f], str(e)[:300], flush=True)
    print(teacher.summary(), flush=True)


if __name__ == '__main__':
    main()
