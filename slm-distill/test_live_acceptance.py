import copy

from verify_live_memory import sister_relationship_is_supported


def fixture():
    graph = {"entities": [{"id": "maya", "name": "Maya Sen"}, {"id": "other", "name": "Other Person"}],
             "assertions": [{"id": "family", "subject_id": "maya", "status": "active",
                             "statement": "The user identifies Leela Sen as their sister.",
                             "value": "Leela Sen is Maya's sister"}]}
    details = {"family": {"sources": [{"quote": "My sister is Leela Sen."}]}}
    return graph, details, ["My name is Maya Sen. My sister is Leela Sen."]


def test_relationship_uses_resolved_subject_and_verbatim_evidence():
    graph, details, sources = fixture()
    assert sister_relationship_is_supported(graph, details, sources)


def test_wrong_subject_is_not_accepted():
    graph, details, sources = fixture()
    graph["assertions"][0]["subject_id"] = "other"
    assert not sister_relationship_is_supported(graph, details, sources)


def test_wrong_or_negated_relationship_is_not_accepted():
    graph, details, sources = fixture()
    for statement in ["Leela Sen is Maya's colleague.", "Leela Sen is not Maya's sister."]:
        changed = copy.deepcopy(graph)
        changed["assertions"][0].update(statement=statement, value=statement)
        assert not sister_relationship_is_supported(changed, details, sources)


def test_unsupported_source_is_not_accepted():
    graph, details, sources = fixture()
    details["family"]["sources"][0]["quote"] = "Leela Sen is my sister."
    assert not sister_relationship_is_supported(graph, details, sources)
