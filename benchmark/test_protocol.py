import json
import tempfile
import unittest
from pathlib import Path

import properties
import run


def bank():
    return {
        "bank": "conv-x",
        "sessions": [
            {
                "external_id": "conv-x:session_1",
                "session_id": "session_1",
                "date": "2023-05-08T13:56:00+00:00",
                "turns": [
                    {"role": "user", "content": "Ann: I went to a support group yesterday.",
                     "metadata": {"dia_id": "D1:1", "speaker": "Ann"}},
                    {"role": "user", "content": "Bo: That sounds powerful.",
                     "metadata": {"dia_id": "D1:2", "speaker": "Bo"}},
                ],
            }
        ],
    }


class ProtocolTests(unittest.TestCase):
    def test_bank_payload_carries_no_question_or_answer(self):
        banks, items = run.load()
        payload = run.retain_payload(Path("runs/x"), items[0]["bank"], banks[items[0]["bank"]]["sessions"][0])
        text = json.dumps(payload)
        for item in items:
            if item["bank"] == items[0]["bank"]:
                self.assertNotIn(item["question"], text)
        self.assertEqual(set(payload), {"namespace", "session_id", "external_id", "metadata", "events"})

    def test_dataset_counts_and_splits(self):
        banks, items = run.load()
        self.assertEqual(len(banks), 10)
        self.assertEqual(len(items), 1540)
        dev_banks, dev_items = run.select("dev", banks, items)
        self.assertEqual(sorted(dev_banks), sorted(banks)[:2])
        self.assertTrue(all(i["bank"] in dev_banks for i in dev_items))
        self.assertEqual(run.select("test", banks, items)[1], items)

    def test_sampling_is_stratified_and_deterministic(self):
        _, items = run.load()
        first = run.stratified(items, 300, "ablation")
        self.assertEqual(first, run.stratified(items, 300, "ablation"))
        self.assertEqual(len(set(first)), 300)
        categories = {i["id"]: i["category"] for i in items}
        counts = {c: sum(categories[i] == c for i in first) for c in set(categories.values())}
        self.assertEqual(set(counts.values()), {75})

    def test_prompts_use_vendored_text(self):
        item = {"question": "Q?", "answer": "A"}
        self.assertIn("Memories:\n\n    MEMORY", run.reader_prompt(item, "MEMORY"))
        rubric = run.judge_prompt(item, "H")
        self.assertIn("Gold answer: A", rubric)
        self.assertIn("Generated answer: H", rubric)
        self.assertNotIn("{", rubric.replace('{"label"', ""))

    def test_mem0_metrics(self):
        self.assertEqual(run.token_f1("7 May 2023", "7 may 2023."), 1.0)
        self.assertEqual(run.token_f1("nothing", "7 May"), 0.0)
        self.assertAlmostEqual(run.bleu1("on 7 May 2023", "7 May 2023"), 0.75)
        self.assertLess(run.bleu1("x", "7 May"), 0.1)

    def test_last_json_takes_the_final_answer(self):
        text = 'Work: {"hypothesis": "draft"}\nFinal:\n{"hypothesis": "7 May 2023"}'
        self.assertEqual(run.last_json(text, "hypothesis"), "7 May 2023")
        self.assertIsNone(run.last_json("no json here", "hypothesis"))

    def test_retrieval_and_quote_checks(self):
        item = {"evidence_sessions": ["session_1"], "evidence_turns": ["D1:1"]}
        source = {"session_id": "session_1", "quote": "I went to a support group",
                  "metadata": {"dia_id": "D1:1"}}
        response = {"ranking": [{"sources": [source]}], "results": [{"sources": [source]}]}
        metrics = run.retrieval_metrics(item, response)
        self.assertEqual(metrics["session_recall_all@5"], 1)
        self.assertEqual(metrics["packed_turn_recall"], 1.0)
        self.assertEqual(run.quote_check(response, {"conv-x": bank()}, "conv-x"), (1, 1))
        source["quote"] = "I went to a party"
        self.assertEqual(run.quote_check(response, {"conv-x": bank()}, "conv-x"), (1, 0))

    def test_paired_ci_and_kappa(self):
        ci = run.paired_ci({"a": 1, "b": 0}, {"a": 0, "b": 0})
        self.assertEqual(ci["difference"], 0.5)
        self.assertEqual(run.cohen_kappa({"a": 1, "b": 0}, {"a": 1, "b": 0})["kappa"], 1.0)

    def test_transcript_is_the_ingested_text(self):
        text = run.transcript(bank())
        self.assertIn("[conv-x:session_1 | 2023-05-08T13:56:00+00:00]", text)
        self.assertIn("Ann: I went to a support group yesterday.", text)

    def test_criteria_follow_the_config(self):
        out = {"primary_judge": "jev", "conditions": {
            "enhanced": {"jev_accuracy": 0.7, "context_tokens_mean": 1000, "quote_exactness": 1.0},
            "no_memory": {"jev_accuracy": 0.0}, "full_context": {"jev_accuracy": 0.75, "context_tokens_mean": 16000}},
            "paired_vs_enhanced": {"jev": {"raw_hybrid": {"ci95": [0.01, 0.05], "cluster_ci95": [-0.01, 0.06]},
                                           "recent_window": {"ci95": [0.1, 0.2], "cluster_ci95": [0.05, 0.25]}}}}
        result = run.criteria(out)
        self.assertTrue(result["beats_no_memory"] and result["not_below_raw_hybrid"] and result["quotes_exact"])
        # The per-conversation interval decides, so a per-question win alone is not enough.
        self.assertFalse(result["above_raw_hybrid"])
        self.assertTrue(result["beats_recent_window"])
        self.assertTrue(result["within_full_context"] and result["token_share_ok"])

    def test_recent_window_keeps_the_newest_turns_within_budget(self):
        banks, _ = run.load()
        bank = banks["conv-26"]
        window = run.recent_window(bank, 2000)
        self.assertLessEqual(len(window), 2000 * 4)
        self.assertTrue(window.rstrip().endswith(bank["sessions"][-1]["turns"][-1]["content"]))
        self.assertGreater(len(window), 2000 * 4 * 0.9)
        self.assertEqual(run.recent_window(bank, 10**7), run.transcript(bank))

    def test_cluster_bootstrap_is_wider_when_conversations_disagree(self):
        # Enhanced wins every question of one conversation and loses every question of the other.
        left = {f"a:q{i}": 1 for i in range(50)} | {f"b:q{i}": 0 for i in range(50)}
        right = {f"a:q{i}": 0 for i in range(50)} | {f"b:q{i}": 1 for i in range(50)}
        ci = run.paired_ci(left, right)
        self.assertEqual(ci["conversations"], 2)
        width = lambda pair: pair[1] - pair[0]
        self.assertGreater(width(ci["cluster_ci95"]), width(ci["ci95"]))

    def test_failed_jobs_are_retried_then_recorded(self):
        calls, jobs = [], [{"id": "j1", "kind": "extract", "status": "failed", "error": "cap"}]

        def fake(base, path, body=None, timeout=300):
            calls.append(path)
            if path.startswith("/api/v2/status"):
                return {"embedding_gaps": {}, "ready": False}
            if path.startswith("/api/v2/jobs?"):
                return jobs
            return {"retry": True}

        real_api, real_sleep = run.api, run.time.sleep
        run.api, run.time.sleep = fake, lambda s: None
        try:
            status = run.wait_ready("http://x", "ns", poll=0)
        finally:
            run.api, run.time.sleep = real_api, real_sleep
        self.assertEqual(sum("/retry" in c for c in calls), 2)
        self.assertEqual(status["permanently_failed_jobs"][0]["id"], "j1")

    def test_dev_run_records_changes_but_test_run_refuses(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            (folder / "source.tar.gz").write_bytes(b"archive")
            for split in ("dev", "test"):
                run.atomic_json(folder / "manifest.json", {
                    "split": split, "item_ids": ["a"], "code_fingerprint": "old",
                    "source_archive_sha256": run.sha(b"archive")})
                if split == "dev":
                    run.prepare(folder, "dev", [{"id": "a"}], None)
                    self.assertEqual(run.read_json(folder / "manifest.json")["later_fingerprints"],
                                     [run.code_fingerprint()])
                else:
                    with self.assertRaisesRegex(RuntimeError, "test run was frozen"):
                        run.prepare(folder, "test", [{"id": "a"}], None)


class RateLimitTests(unittest.TestCase):
    def setUp(self):
        # A fake clock that sleeping advances, so waits are counted instead of taken.
        self.sleeps, self.clock = [], [1000.0]
        self.real = run.time.sleep, run.time.monotonic

        def sleep(seconds):
            self.sleeps.append(seconds)
            self.clock[0] += seconds

        run.time.sleep, run.time.monotonic = sleep, lambda: self.clock[0]
        run.GATE.until.clear()

    def tearDown(self):
        run.time.sleep, run.time.monotonic = self.real
        run.GATE.until.clear()

    def test_rate_limit_waits_and_retries_without_spending_attempts(self):
        calls = []

        def flaky():
            calls.append(1)
            if len(calls) <= 5:
                raise run.RateLimited("jev", "HTTP 429", retry_after=2 if len(calls) == 1 else None)
            return "ok"

        self.assertEqual(run.retry(flaky, "jev", attempts=2), "ok")
        self.assertEqual(len(calls), 6)
        # The first wait honours Retry-After, the rest back off exponentially (30, 60, 120, 240 s).
        self.assertAlmostEqual(sum(self.sleeps), 2 + 30 + 60 + 120 + 240)

    def test_rate_limit_budget_is_finite(self):
        def always():
            raise run.RateLimited("reader", "usage limit")

        with self.assertRaises(run.RateLimited):
            run.retry(always, "reader", limit_budget=100)

    def test_ordinary_errors_stop_after_attempts(self):
        def broken():
            raise RuntimeError("bad output")

        with self.assertRaisesRegex(RuntimeError, "bad output"):
            run.retry(broken, "reader", attempts=3)
        self.assertEqual(self.sleeps, [20, 40])

    def test_subscription_limit_text_is_recognised(self):
        for text in ("Claude AI usage limit reached|1760970000", "rate_limit_error", "Error 529 overloaded",
                     "429 Too Many Requests"):
            self.assertTrue(run.LIMIT_TEXT.search(text), text)
        self.assertFalse(run.LIMIT_TEXT.search("reader gave no JSON hypothesis"))

    def test_http_429_and_5xx_are_rate_limits(self):
        import io
        import urllib.error

        def failing(code, headers):
            def opener(request, timeout):
                raise urllib.error.HTTPError(request.full_url, code, "x", headers, io.BytesIO(b"slow down"))
            return opener

        real = run.urllib.request.urlopen
        try:
            run.urllib.request.urlopen = failing(429, {"retry-after": "7"})
            with self.assertRaises(run.RateLimited) as caught:
                run.http_json("jev", "https://example.invalid", {}, {})
            self.assertEqual(caught.exception.retry_after, 7.0)
            run.urllib.request.urlopen = failing(400, {})
            with self.assertRaises(RuntimeError) as caught:
                run.http_json("jev", "https://example.invalid", {}, {})
            self.assertNotIsInstance(caught.exception, run.RateLimited)
        finally:
            run.urllib.request.urlopen = real


class PropertySuiteTests(unittest.TestCase):
    def test_suite_is_fixed_by_the_seed(self):
        first = properties.scenarios(run.CONFIG["seed"])
        self.assertEqual(first, properties.scenarios(run.CONFIG["seed"]))
        self.assertEqual(len({s["id"] for s in first}), len(first))

    def test_relative_dates_have_the_right_expected_day(self):
        for s in properties.scenarios(run.CONFIG["seed"]):
            if s["family"] == "relative_date":
                said = s["sessions"][0]["date"][:10]
                delta = dict(properties.RELATIVE)[s["relative"]]
                expected = properties.dt.date.fromisoformat(said) + properties.dt.timedelta(days=delta)
                self.assertEqual(s["expect_date"], expected.isoformat())
                self.assertIn(s["relative"], s["sessions"][0]["turns"][1])


if __name__ == "__main__":
    unittest.main()
