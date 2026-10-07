import unittest
import tempfile
from pathlib import Path
import run


class ProtocolTests(unittest.TestCase):
    def test_repeated_session_ids_keep_distinct_episode_identity(self):
        q = {
            "haystack_session_ids": ["s", "s"],
            "haystack_dates": ["2023/05/20 (Sat) 02:21"] * 2,
            "haystack_sessions": [
                [{"role": "user", "content": "first"}],
                [{"role": "user", "content": "second"}],
            ],
        }
        first = run.ingestion_payload(q, 0, "bank")
        second = run.ingestion_payload(q, 1, "bank")
        self.assertEqual(first["session_id"], second["session_id"])
        self.assertNotEqual(first["external_id"], second["external_id"])
        self.assertEqual(first, run.ingestion_payload(q, 0, "bank"))

    def test_audit_regeneration_preserves_reviews_and_rejects_changed_answers(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            run.atomic_json(folder / "manifest.json", {"repeat_sample": ["q"]})
            question = {
                "question_id": "q",
                "question": "Which language?",
                "answer": "Rust",
                "question_type": "single-session-user",
            }
            answer = folder / "answers" / run.CONFIG["conditions"][0] / "0/q.json"
            run.atomic_json(answer, {"hypothesis": "Rust"})
            run.audit_packet(folder, [question])
            packets = run.json.loads((folder / "audit/packet.json").read_text())
            packets[0]["reviewer_1"] = True
            run.atomic_json(folder / "audit/packet.json", packets)
            run.audit_packet(folder, [question])
            self.assertTrue(
                run.json.loads((folder / "audit/packet.json").read_text())[0][
                    "reviewer_1"
                ]
            )
            run.atomic_json(answer, {"hypothesis": "Python"})
            with self.assertRaisesRegex(RuntimeError, "audit content changed"):
                run.audit_packet(folder, [question])

    def test_ingestion_excludes_ground_truth(self):
        q = {
            "question": "SECRET QUESTION",
            "answer": "SECRET ANSWER",
            "answer_session_ids": ["evidence"],
            "haystack_session_ids": ["s"],
            "haystack_dates": ["2023/05/20 (Sat) 02:21"],
            "haystack_sessions": [
                [{"role": "user", "content": "I use Rust", "has_answer": True}]
            ],
        }
        payload = run.ingestion_payload(q, 0, "bank")
        text = run.json.dumps(payload)
        for forbidden in ("SECRET", "has_answer", "answer_session_ids"):
            self.assertNotIn(forbidden, text)
        self.assertEqual(
            payload["events"][0]["occurred_at"], "2023-05-20T02:21:00+00:00"
        )

    def test_multisession_requires_all_sources(self):
        q = {"question_id": "q", "answer_session_ids": ["a", "b"]}
        response = {"ranking": [{"sources": [{"session_id": "a"}]}], "results": []}
        metrics = run.retrieval_metrics(q, response)
        self.assertEqual(metrics["recall_all@5"], 0)
        self.assertEqual(metrics["recall_fraction@5"], 0.5)
        q["question_id"] = "q_abs"
        self.assertIsNone(run.retrieval_metrics(q, response))

    def test_sampling_and_paired_ci(self):
        data = [
            {"question_id": str(i), "question_type": "a" if i % 2 else "b"}
            for i in range(200)
        ]
        self.assertEqual(
            run.sample_questions(data, 100), run.sample_questions(data, 100)
        )
        self.assertEqual(len(set(run.sample_questions(data, 100))), 100)
        ci = run.paired_ci({"a": True, "b": False}, {"a": False, "b": False})
        self.assertEqual(ci["accuracy_difference"], 0.5)


if __name__ == "__main__":
    unittest.main()
