#!/usr/bin/env python3
"""Live local-SLM check of entities, contested reports, correction history and projection.

Creates a separate diagnostic namespace and retains a JSON receipt. Never clears user data.
"""
import argparse
import datetime as dt
import json
from pathlib import Path
import time
import urllib.parse
import urllib.request
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', default='http://127.0.0.1:18080')
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--timeout', type=int, default=300)
    args = parser.parse_args()
    namespace = 'diagnostic:entity-conflict:' + str(uuid.uuid4())
    query = '?namespace=' + urllib.parse.quote(namespace)
    receipt = {'namespace': namespace, 'steps': {}, 'checks': {}}
    args.out.parent.mkdir(parents=True, exist_ok=True)

    def save():
        args.out.write_text(json.dumps(receipt, indent=2) + '\n')

    def request(route, body=None):
        req = urllib.request.Request(args.url.rstrip('/') + route,
            data=None if body is None else json.dumps(body).encode(),
            headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=60) as response:
            return json.load(response)

    def check(name, value):
        receipt['checks'][name] = bool(value)
        save()
        if not value:
            raise AssertionError(name)
        print('PASS', name, flush=True)

    def retain(stage, text):
        print('Retaining', stage, flush=True)
        receipt['steps'][stage] = {'text': text, 'retain': request('/api/v2/retain', {
            'namespace': namespace, 'session_id': 'entity-conflict-check',
            'external_id': stage, 'events': [{'role': 'user', 'content': text,
                'occurred_at': dt.datetime.now(dt.timezone.utc).isoformat()}]})}
        save()
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            jobs = request('/api/v2/jobs' + query)
            receipt['steps'][stage]['jobs'] = jobs
            save()
            if any(job['status'] == 'failed' for job in jobs):
                raise RuntimeError('Worker job failed; see receipt')
            if jobs and all(job['status'] == 'succeeded' for job in jobs):
                graph = request('/api/v2/graph' + query)
                receipt['steps'][stage]['graph'] = graph
                save()
                return graph
            time.sleep(2)
        raise TimeoutError('Jobs did not finish; see receipt')

    def assertions_with(graph, value):
        return [a for a in graph['assertions']
                if a['kind'] != 'inference' and a['value'].lower() == value.lower()]

    try:
        receipt['health'] = request('/healthz')
        initial = retain('initial', "Project Orion's database is PostgreSQL.")
        old = assertions_with(initial, 'PostgreSQL')
        check('initial_assertion_active', old and old[0]['status'] == 'active')
        old = old[0]
        check('database_entity_extracted', any(e['name'] == 'PostgreSQL' and e['entity_type'] == 'technology'
                                                for e in initial['entities']))
        conflict = retain('conflict', "Project Orion's database is MySQL.")
        new = assertions_with(conflict, 'MySQL')
        check('both_conflicting_values_contested', new and new[0]['status'] == 'contested'
              and assertions_with(conflict, 'PostgreSQL')[0]['status'] == 'contested')
        new = new[0]
        check('subject_identity_preserved', old['subject_id'] == new['subject_id'])
        check('contradiction_edge', any(e['from_id'] == new['id'] and e['to_id'] == old['id']
                                       and e['relation'] == 'contradicts' for e in conflict['edges']))
        correction = retain('correction', "Correction: Project Orion's database is SQLite, not PostgreSQL or MySQL.")
        current = assertions_with(correction, 'SQLite')
        check('explicit_correction_active', current and current[0]['status'] == 'active')
        current = current[0]
        check('both_old_values_superseded', all(assertions_with(correction, v)[0]['status'] == 'superseded'
              and assertions_with(correction, v)[0]['valid_to'] is not None for v in ['PostgreSQL', 'MySQL']))
        check('supersession_edges', all(any(e['from_id'] == current['id'] and e['to_id'] == a['id']
              and e['relation'] == 'supersedes' for e in correction['edges']) for a in [old, new]))
        entities = retain('entities', 'Mira owns Project Orion at Cedar Labs.')
        names = {(e['name'], e['entity_type']) for e in entities['entities']}
        check('person_project_organization_entities', {
            ('Mira', 'person'), ('Project Orion', 'project'), ('Cedar Labs', 'organization')}.issubset(names))
        details = request('/api/v2/assertions/' + current['id'] + query)
        receipt['correction_detail'] = details
        check('correction_quotes_grounded', details['sources'] and all(
            source['quote'] and source['quote'] in receipt['steps']['correction']['text']
            for source in details['sources']))
        projection = request('/api/v2/graph/projection' + query)
        receipt['projection'] = projection
        projected = {node['id']: node for node in projection['nodes']}
        check('neo4j_matches_history', projected[current['id']]['status'] == 'active'
              and all(projected[a['id']]['status'] == 'superseded' for a in [old, new]))
        receipt['passed'] = True
    except Exception as error:
        receipt['passed'] = False
        receipt['error'] = str(error)
    finally:
        save()
    print(json.dumps({'namespace': namespace, 'passed': receipt['passed'], 'receipt': str(args.out),
                      'error': receipt.get('error')}), flush=True)
    raise SystemExit(0 if receipt['passed'] else 1)


if __name__ == '__main__':
    main()
