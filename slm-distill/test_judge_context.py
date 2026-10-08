"""The judge needs the same attribution and date evidence as the extractor."""
import json

import judge


def test_judge_receives_event_dates_metadata_and_previous_facts():
    captured = {}

    class Teacher:
        def complete_json(self, system, user, **kwargs):
            captured['user'] = user
            return {'claims': [], 'planted': []}

    event = {'role': 'user', 'content': 'Move the demo to September 22.',
             'occurred_at': '2026-09-20T13:00:01Z', 'metadata': {'cwd': '/work/gatehouse'}}
    existing = [{'subject': 'gatehouse', 'predicate': 'demo_date', 'value': 'September 21, 2026'}]
    judge.grade_window(Teacher(), [event], [], [], 'probe', existing=existing)
    # Prior facts establish the subject, and event time establishes the year.
    assert json.dumps(event['occurred_at']) in captured['user']
    assert json.dumps(event['metadata']) in captured['user']
    assert json.dumps(existing) in captured['user']
