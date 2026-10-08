"""Ollama wire schema shared with src/model/structured.rs; validators stay unchanged."""
import json
import re
from pathlib import Path
import prompt

ROOT = Path(__file__).resolve().parents[1]/'src'/'model'
WIRE_INSTRUCTION = (ROOT/'paired_sources.txt').read_text().rstrip()
S = {'type': 'string'}

def obj(properties):
    return {'type':'object','properties':properties,'required':list(properties),'additionalProperties':False}

def quote_candidates(content):
    spans = re.split(r'\n\[\.\.\. \d+ characters omitted \.\.\.\]\n', content)
    if len(spans) > 1:
        return list(dict.fromkeys(q for span in spans for q in quote_candidates(span)))
    candidates = [content, *content.splitlines()]
    start = 0
    previous = ' '
    for index, char in enumerate(content):
        if char.isspace() and previous in '.!?':
            candidates.append(content[start:index]); start = index
        previous = char
    candidates.append(content[start:].lstrip())
    return list(dict.fromkeys(s.lstrip() for s in candidates if s.strip() and 'characters omitted' not in s))

def first_json(text):
    start = len(text)-len(text.lstrip())
    value, end = json.JSONDecoder().raw_decode(text, start)
    return value, text[end:]

def prepare(user):
    if user.startswith('You are the extraction worker'):
        existing, tail = first_json(user.split('\nExisting facts: ',1)[1])
        tail = tail.lstrip()
        if not tail.startswith('Events: '): raise ValueError('missing events')
        events, _ = first_json(tail[len('Events: '):])
        branches = []
        for entry in events:
            quotes = quote_candidates(entry['event']['content'])
            if quotes:
                branches.append(obj({'source_index':{'const':entry['source_index']},'quote':{'enum':quotes}}))
        ids = [f['id'] for f in existing]
        related = {'type':'array','items':obj({'assertion_id':{'enum':ids},'relation':{'enum':['extends','contradicts','causes']},'explanation':S})} if ids else {'type':'array','items':{'type':'object'},'maxItems':0}
        schema = json.loads(prompt.fill((ROOT/'extract_schema.json').read_text(),{'SOURCES':json.dumps(branches),'RELATED':json.dumps(related)})) if branches else obj({'claims':{'type':'array','items':{'type':'object'},'maxItems':0}})
        return user+'\n'+WIRE_INSTRUCTION, schema, True
    if user.startswith('Consolidate evidence into'):
        facts, _ = first_json(user.split(' Facts: ',1)[1])
        def ids(key):
            values = [f[key] for f in facts]
            return {'enum':values} if values else S
        schema=json.loads(prompt.fill((ROOT/'consolidate_schema.json').read_text(),{'SUBJECTS':json.dumps(ids('subject_id')),'IDS':json.dumps(ids('id'))}))
        return user, schema, False
    return user, 'json', False

def canonicalize(value, paired):
    if paired:
        for claim in value['claims']:
            if 'source_indices' in claim or 'quotes' in claim: raise ValueError('mixed source formats')
            pairs = claim.pop('source_quotes')
            if not isinstance(pairs,list): raise ValueError('source pairs must be array')
            if any(type(p['source_index']) is not int or p['source_index'] < 0 or not isinstance(p['quote'],str) for p in pairs): raise ValueError('invalid source pair')
            claim['source_indices']=[p['source_index'] for p in pairs]
            claim['quotes']=[p['quote'] for p in pairs]
    return value

def training_row(row):
    user, _, paired = prepare(row['prompt'])
    target = json.loads(row['target'])
    if paired:
        for i,c in enumerate(target['claims']):
            refs=[{'source_index':idx,'quote':q} for idx,q in zip(c.pop('source_indices'),c.pop('quotes'),strict=True)]
            target['claims'][i]={'source_quotes':refs,**c}
    return {**row,'prompt':user,'target':json.dumps(target,ensure_ascii=False)}
