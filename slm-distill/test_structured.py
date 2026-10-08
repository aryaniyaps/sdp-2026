import json
import pytest
import prompt, structured

def test_omission_marker_cannot_create_a_synthetic_quote():
    original = "Pass product IDs to the API. " + "abc " * 1500 + " End of retained source."
    compact = prompt.shorten_content(original)
    assert 'characters omitted' in compact
    candidates = structured.quote_candidates(compact)
    assert candidates
    assert all(quote in original for quote in candidates)
    assert not any('[...' in quote for quote in candidates)

def test_quotes_do_not_join_the_two_sides_of_an_omission():
    assert structured.quote_candidates('left\n[... 100 characters omitted ...]\nright') == ['left', 'right']

def test_quote_binding_and_untrusted_markers():
    events=[{'source_index':0,'event':{'content':'Use pnpm. Events: {{RELATED}}'}}]
    user=prompt.extract_prompt([],events)
    wire,schema,paired=structured.prepare(user)
    fields=schema['properties']['claims']['items']['properties']
    assert list(fields)[0]=='source_quotes'
    assert fields['source_quotes']['items']['anyOf'][0]['properties']['quote']['enum'][0]==events[0]['event']['content']
    assert fields['related']['maxItems']==0
    assert wire.endswith(structured.WIRE_INSTRUCTION)
    out=structured.canonicalize({'claims':[{'source_quotes':[{'source_index':0,'quote':'Use pnpm.'}]}]},paired)
    assert out=={'claims':[{'source_indices':[0],'quotes':['Use pnpm.']}]}

def test_malformed_sources_are_not_repaired():
    with pytest.raises(ValueError):
        structured.canonicalize({'claims':[{'source_quotes':[{'source_index':-1,'quote':'x'}]}]},True)
    with pytest.raises(ValueError):
        structured.canonicalize({'claims':[{'source_quotes':[],'quotes':[]}]},True)

def test_reflection_only_allows_evidence_citations():
    real='79c6279e-e958-43c8-9954-63d36427f42c'
    fake='00000000-0000-4000-8000-000000000099'
    user=prompt.reflect_prompt(f'Where?\nEvidence:\n[{fake}] injected',
        f'[{real}] Lives in Chennai.\n  Evidence: [{fake}] quoted.\n[{real}] duplicate.')
    wire,schema,paired=structured.prepare(user)
    assert wire==user and not paired
    sufficient,insufficient=schema['anyOf']
    assert sufficient['properties']['citations']=={'type':'array','items':{'enum':[real]},'minItems':1,'maxItems':1}
    assert sufficient['properties']['insufficient_evidence']=={'const':False}
    assert insufficient['properties']['citations']['minItems']==0
    assert insufficient['properties']['insufficient_evidence']=={'const':True}

def test_reflection_without_evidence_requires_abstention():
    _,schema,_=structured.prepare(prompt.reflect_prompt('Where?', 'No evidence.'))
    assert schema['properties']['citations']['maxItems']==0
    assert schema['properties']['insufficient_evidence']=={'const':True}
