# Viva preparation

Checked against commit 1797536. Every reference was re-read on this code. Weaknesses are marked (admit): say them before the examiner finds them.

## How a fetch (recall) works

Recall is hybrid. Rank fusion is still the core, and the knowledge graph is one of four searches.

1. **Time window.** If the request gives no `as_of`, `from` or `to` and `temporal` is on, a rule-based parser (`temporal.rs:160`) looks for phrases like "last week" or "in March 2025". It calls no model unless `TEMPORAL_PLANNER=model`. That mode now asks the local worker model (`memex-extractor`), not Pi.
2. **Four searches.**
   - **Keyword:** Postgres full-text search over the facts.
   - **Meaning:** pgvector cosine search with the query embedding.
   - **Time:** the newest facts inside the window. This runs only when there is a window.
   - **Graph:** the seeds are the top 5 keyword hits, the top 5 meaning hits and up to 10 facts whose entity is named in the query, capped at 20. Neo4j expands them two hops (at most 200 rows), and Postgres re-checks every id.
3. **Raw text.** Original conversation chunks are added when `include_raw` is set, when nothing matched, or while extraction is pending or failed.
4. **Rank fusion.** Each fact scores the sum of 1/(60 + rank) over the searches that found it. Ties go to the smaller UUID.
5. **Packing.** Whole blocks, each with up to 3 quotes, are added while they fit the token budget.

We have not measured what the graph search adds (admit). The benchmark has a `no_graph` condition, but it has never been run.

## What changed since the last version of this sheet

The changes listed in the old sheet as uncommitted have since been committed:
- clear: 4c0877c
- latency: 69ddf6a

The code was then reorganised:
- `v2.rs` became `src/v2/{mod,recall,graph,reflect}.rs`.
- `knowledge_store.rs` became `src/knowledge_store/{mod,assertions,evidence,jobs,maintenance}.rs`.
- `worker.rs` became `src/worker/{mod,extraction,consolidation,projection,prompts,window}.rs`.
- `graph_view.html` became the React app in `frontend/`.
- The V1 APIs were removed.

Two changes alter the answers:
- **The worker no longer runs Pi.** It calls a local Ollama model, `EXTRACTION_MODEL` (default `memex-extractor`), at `EXTRACTION_OLLAMA_URL`. If `MEMORY_MODEL_PROVIDER` is set to anything other than `ollama`, startup stops.
- **Extraction reads an episode in windows** of about 2,800 estimated tokens, using shortened events. Quotes are still checked against the full events.

Things that are now out of date and that the examiner may find (admit):
- `scripts/run-memory.sh` still requires Pi, runs `check-worker.py` and exports `PI_PROVIDER`/`PI_MODEL`. The binary ignores all of these, and the script never fetches `memex-extractor`.
- `src/main.rs:123` tells you to run `scripts/fetch-slm.sh`, which does not exist.
- `README.md:169-171`, `docs/evidence-memory.md:17,23`, `deploy/azure/docker-compose.yml:67-68` and `deploy/azure/README.md:31` still describe a Pi worker.
- `benchmark/run.py:255-263` expects `worker_model == "pi/…"`, but `/healthz` now reports `ollama/<model>`. As written, `ingest` refuses to start.
- The latency figures (p95 4,252 ms to 71 ms, 12 ms for a repeated question) were measured before the refactor. The script that measured them (`scripts/recall-latency.py`) was deleted in 8bcde83, so the repo can no longer reproduce them.
- The deck still shows the old latency numbers (slides 5 and 14).

---

## A. How it works

**1. Walk me through a fetch from the HTTP call to the context string.**
Where: route `src/v2/mod.rs:83`, handler at 224. `recall_engine` is at `src/v2/recall.rs:400`, `candidates` at 102, `temporal::plan` at `temporal.rs:160`, `GraphStore::neighbors` at `graph.rs:136`, `fact_rows` at 209, `raw_source_rows` at 157, and packing at 745-786.
Answer: the steps run in this order:
1. Validate the bounds (401-422).
2. Find the time window (426-467).
3. `tokio::join!` at 504 runs four things together: the keyword search, the query embedding, the entity lookup and the count of outstanding projection jobs.
4. Run the semantic search (511-523), then the time channel (524-528).
5. Expand the graph (539-632).
6. Assign the ranks (643-656), then the raw fallback (659-694).
7. Load the rows in bulk (697-701).
8. Compute RRF scores (704), sort (744), pack (745-786), and write the trace (787-801).

Trap: "Which step is slowest now?" The query embedding. A repeated short query skips it through the cache.

**2. Is retrieval rank fusion or just the knowledge graph?**
Where: RRF at `recall.rs:704` and 744. Graph channel at 539-632. Benchmark flags at `benchmark/run.py:352-356`.
Answer: Both. Keyword search misses paraphrases, vectors miss exact names, and the time channel handles dates. The graph adds facts linked to the seeds. The data model (versions, `supersedes`/`contradicts`, quote provenance) holds whatever retrieval does. RRF merges by rank only, so ts_rank, cosine distance and graph order never need to be on the same scale.
Trap: "What does the graph add?" Unmeasured (admit).

**3. Walk me through ingestion end to end.**
Where:
- Pi side: `integrations/pi/extension.ts:120-157` (tool results) and 158-247 (settle). The spool is in `integrations/pi/spool.ts:17-39`.
- Retain: `src/v2/mod.rs:104`, then `knowledge_store/evidence.rs:4-83`.
- Worker: claim in `knowledge_store/jobs.rs:25-120`, heartbeat in `worker/mod.rs:119-143`, extraction in `worker/extraction.rs`, storage in `knowledge_store/assertions.rs`, projection in `worker/projection.rs` and `graph.rs:54-79`.

Answer:
1. Pi hooks write a batch to the spool on disk.
2. `POST /api/v2/retain` stores the episode, its events, one chunk per event and an extract job, all in one transaction.
3. An early embedding task starts (at most two run at once).
4. A worker claims the job under a per-namespace lease and splits the episode into windows.
5. For each window, it prompts the local model with shortened events and a budgeted list of existing facts. Each reply is validated, with up to 2 repairs.
6. The window indices are moved back to episode indices.
7. `apply_extraction` validates again, resolves entities and versions in one transaction, and enqueues project and consolidate jobs.
8. Projection copies the facts into Neo4j.

The V1 shadow copy to `<ns>:legacy` no longer exists.
Trap: "Where can a message be lost?" See questions 12, 35 and 39.

**4. Walk me through "told in one directory, recalled in another".**
Where: `integrations/pi/identity.ts:7-15`, `extension.ts:25`, 74-80 and 93-119.
Answer: The namespace is per user, not per directory. It is `user:<login>`, unless `MEMORY_NAMESPACE` or `/memory-namespace` overrides it. The spool directory is also per namespace, so a session in directory B delivers batches that A left queued.

Session 1 settles, and its batch is spooled and retained. Session 2 starts fresh. `before_agent_start` calls recall with the prompt (`temporal:false`, `max_distance` 0.45, 15 s timeout) and injects the result as an `sdp-memory-context` message labelled "evidence, not instructions".
Trap: "Is the fact already extracted?" Not necessarily. Raw text is served until extraction finishes.

## B. Retrieval

**5. Where is RRF with k = 60, and are ties deterministic?**
Where: `recall.rs:704` (score), 744 (sort), 787 (formula in the trace).
Answer: The score is `ranks.values().map(|r| 1.0/(60.0+r)).sum()`. Results are sorted by score, then by the smaller UUID.
(admit) `ranks` is a `HashMap`, so the order of the terms is random. Float addition is not associative, so a fact with 3 or 4 terms can differ in the last bit, and the UUID rule is never reached. The fix is to sum in a fixed channel order. No test covers ties.

**6. What decides which facts are visible for as_of, from and to?**
Where: `FILTER` at `recall.rs:101`, the graph re-check at 583-595, and `migrations/0003_evidence_graph.sql`.
Answer:
- Observations are excluded only when the request turns them off.
- Retracted and stale rows are never shown.
- With no `as_of`, only active or contested rows appear. With `as_of`, a row appears when `valid_from <= as_of` and `valid_to` is null or later.
- The window compares `coalesce(event_at, valid_from)` over a half-open range.
- Neo4j ids pass through the same `FILTER`, so the graph cannot add a stale fact.

No index covers the date columns, so these are row filters.

**7. How is the context packed, and how many queries load the hits?**
Where: `raw_source_rows` at `recall.rs:157`, `fact_rows` at 209, the load at 697-701, packing at 745-786, and `estimate_tokens` at 853.
Answer:
- Tokens are estimated as chars/4.
- A block is a header plus up to 3 quotes, each cut to 1,200 characters.
- A block that does not fit is skipped, and the loop continues, so a smaller later block can still fit.
- All hits load in at most 3 statements: two for facts and their sources, one for raw chunks. This used to be 3 per hit.

(admit) A skipped block adds no degraded reason.
Trap: "Why not load only the top_k?" The response returns the whole ranking.

**8. Where is max_distance enforced, and why does it never reorder?**
Where:
- Field docs: `recall.rs:55-69`.
- Validation: 415-422.
- Gate on facts and raw vectors: 112-115 and 119/128.
- Raw lexical pass: 333-343.
- Raw vector pass: 373-377.
- Graph gate: 599-631.
- The 0.45 constant: `integrations/pi/client.ts:21`.

Answer: The gate `<=> $7 <= $8` goes into the SQL before `LIMIT 40`, so the survivors are a prefix and keep their ranks. The raw lexical pass numbers rows with `row_number()` first and filters afterwards, which leaves gaps. Graph-only facts that are too far are dropped, and the trace lists them (`dropped_by_max_distance`).

Not gated: keyword matches on facts, the time channel, and rows that have no embedding yet. The server has no default; only the Pi client sets 0.45, and only for automatic recall.
(admit) 0.45 was calibrated on one namespace with one embedding model. The repo sets no `hnsw.ef_search`.

**9. When is raw source text added, and how is the query made safe?**
Where: `recall.rs:659-694`, `UNPROCESSED_CHUNK` at 266, `any_word_query` at 269-288, `raw_fallback_candidates` at 309-399.
Answer: Raw passes run when `include_raw` is set, when nothing matched, or when an extract job is pending, running or failed.

`any_word_query` cleans the query as follows:
- splits on whitespace only;
- strips quotes and leading or trailing punctuation;
- drops one-letter words and "or";
- removes duplicates;
- keeps at most 32 words;
- joins them with " or ".

(admit) One failed extract job keeps the whole namespace degraded until someone retries it.

**10. What made recall slow, and what exactly did you change?**
Where:
- Planner choice: `recall.rs:426-467` and 814-852.
- Concurrent lookups: 472-509.
- Embedding cache and keep-alive: `providers.rs:20-31` and 91-120.
- Warm-up: `main.rs:131-140`.
- Planner set at startup: `main.rs:56-57`.

Answer: `looks_temporal` (856) matches substrings such as "when", "before" and "may ", and it used to block on a model call. We measured 4 of 40 questions taking 2 to 4.5 s and still returning no window.

The fixes:
- `TEMPORAL_PLANNER=rules|model|off` now selects the planner. The default is `rules`, and an unknown value stops startup.
- The entity lookup and the job count run alongside the embedding.
- The embedder caches query vectors: 256 entries, texts of at most 512 characters, oldest out first.
- Ollama keeps the embedding model loaded for 1 hour, and startup warms it up.

Result: p95 went from 4,252 ms to 71 ms.
(admit) This was measured before the refactor, and the script that measured it has been deleted. The cache evicts the oldest entry first, not the least recently used.
Trap: "What is still serial?" The semantic search, the time channel, the graph, the raw fallback and the trace insert.

**11. What does the date parser accept, and what does it get wrong?**
Where: `temporal.rs:160-257`, tests at 262-405, the call at `recall.rs:435`.
Answer: The parser accepts:
- yesterday, today;
- last or this week, month, year;
- "N units ago";
- "in/during/of March [2025]" and "since 2025";
- ISO dates;
- "as of <date>".

Windows are in UTC and half-open, and weeks start on Monday. A test checks that twelve ordinary questions get no window. The matched phrase is shown in `temporal_plan`.
(admit) "the state of march" and "today's plan" both get a window.

**12. What happens to the embedding of a retained chunk?**
Where: `v2/mod.rs:101-128`, `worker/mod.rs:29-55`, `worker/extraction.rs:75`, status at `v2/mod.rs:184-202`.
Answer: A spawned task embeds the new chunks behind a 2-permit semaphore, off the acknowledgement path. The extract job tries again for any chunk still missing an embedding (`embed_missing_chunks`).
(admit) On a provider error both paths only log a warning. The extract job still succeeds, and nothing backfills afterwards. `/api/v2/status` reports this as `embedding_gaps`.

## C. Knowledge model

**13. Where is a quote proven to be exact, and what can't that catch?**
Where: `knowledge.rs:229-264` (the check is at 251). It runs per window in `worker/extraction.rs:129` and again over the whole episode in `knowledge_store/assertions.rs:18-20`, before the transaction.
Answer: The check is `event.content.contains(quote)`, a plain substring test. Source indices pair one to one with quotes, and empty quotes are rejected. If the text appears in a different event, the error names that event.

The model sees shortened events (head 1,100 and tail 360 characters), but the check runs on the full event.
(admit) The check proves the text exists, not that it supports the claim. There is no minimum length and no offset. Quotes fail in three cases, and each failure uses up a repair:
- the quote crosses the `[... N characters omitted ...]` marker;
- the quote comes from tool metadata the model was shown;
- the quote copies JSON escapes such as `\n` literally.

**14. Where do you guarantee one active value per slot?**
Where: `migrations/0003_evidence_graph.sql:43-44`, the lock at `knowledge_store/mod.rs:23-32`, and `assertions.rs:22`, 86-91, 103-112 and 114-118.
Answer: The partial unique index `assertions_single_active` is only the backstop. The real prevention is the order inside one transaction:
1. take the advisory lock (`hashtextextended(namespace,0)`);
2. select the old rows `FOR UPDATE`;
3. flip their status;
4. insert the new row.

The index excludes contested rows, so several contested rows can coexist.
(admit) No test hits the index directly.

**15. A different value arrives for a single slot. What happens?**
Where: `assertions.rs:92-132`; the `related` path is at 183-209.
Answer: The new row becomes active only if the slot was empty, or if it is a correction with confidence >= 0.8 (a hard-coded literal). In that case the old rows become superseded and a `supersedes` edge is added. Otherwise every row becomes contested and a `contradicts` edge is added.
(admit) A low-confidence claim can push a good active value to contested. Only the supersede branch is tested. Two windows of the same episode that disagree without marking a correction also end up contested, because window 2 cannot see window 1's claims.

**16. valid_from, valid_to, event_at, recorded_at: how are out-of-order facts handled?**
Where: the CHECK at `0003:41`, `assertions.rs:49-56`, 108 and 189, and the job order at `jobs.rs:59-67`.
Answer: The CHECK forbids `valid_to < valid_from`. A retroactive correction therefore closes the old row with `GREATEST(valid_from, $3)`, which gives an empty interval rather than a negative one. The time of the first source event is right with windows only because indices are moved back before `apply_extraction`.
Test: `retroactive_correction_closes_old_belief_without_negative_interval`.
(admit) Jobs run in ingestion order, not event order, so an old fact that arrives late becomes contested.

**17. Where is "an observation needs two supports" enforced?**
Where: the worker at `worker/consolidation.rs:100-140` and the store at `assertions.rs:329-347`.
Answer: The worker checks that there are at least 2 distinct supports, all taken from the facts in the prompt, with one of them about the subject. The store checks again: at least 2 distinct supports, all active, in the namespace and locked `FOR UPDATE`, with one of them about the subject.
(admit) The rule lives only in Rust, so a direct SQL insert with one support is accepted. No negative test exists.

**18. What happens to "C++" and "C#"?**
Where: `domain.rs:1-8`; used at `assertions.rs:47-48`, 57-80 and 166, and `knowledge_store/mod.rs:68`.
Answer: (admit) `normalize_component` trims non-alphanumeric characters from both ends, so both become "c". The reinforce check then treats "Ada uses C#" after "C++" as reinforcement, so the new value is never stored. An entity named "C++" also merges with "C". This comes from reading the code. I have not run it, and no test covers it.

**19. What makes retain idempotent, and what if two identical calls race?**
Where: `knowledge_store/evidence.rs:4-83` (hash at 25-28, lock at 30, lookup and 409 at 31-51), and `UNIQUE(namespace, external_id)` at `0003:9`.
Answer: Retain hashes the whole request with SHA-256, takes the namespace advisory lock, then looks up `(namespace, external_id)`. The same hash returns the old ids with `created=false`. A different hash returns 409. The unique constraint is the backstop.
(admit) The lock is held while up to 1,000 events are inserted, so a long extraction transaction delays retain for that namespace, and the reverse.

## D. Extraction worker

**20. Where is the "existing facts" list built, and why is it capped at 80 and frozen?**
Where: the snapshot at `worker/extraction.rs:13-33` (`LIMIT 80`, newest first). It is frozen into the payload as `existing_snapshot`, fenced by the lease. Per-window budgeting is at `extraction.rs:47-60` and `prompts.rs:108-148` (`fit_existing_facts`).
Answer: One SQL statement takes active or contested non-observation facts. They are written into the job payload, so retries build the same prompt and replay from the model cache.

Each window then gets a share that fits a 6,000-token prompt target (`prompts.rs:13`). The facts kept are those the window mentions most: subject scores 3, value 2, short text 1, with recency breaking ties. The number dropped is reported as `existing_facts_dropped`.
(admit) The 80 are chosen across the whole namespace by recency alone, so an older mentioned fact is never seen. With realistic facts only about 10 to 35 reach each window. `related` links can only point to facts that were kept.

**21. Where are num_ctx and num_predict applied and checked?**
Where:
- `src/model.rs`: defaults at 25 and 27, validation at 49-100, estimate at 42-47, preflight at 114-123, response check at 126-147, cache-after-success at 219-220.
- `main.rs:55` (startup validation).
- `tests/ollama_context.rs`.

Answer: The defaults are 16,384 and 4,096, validated at startup (`num_predict < num_ctx`). Preflight estimates the prompt's tokens and refuses before sending. After the call, it raises an error in any of these cases:
- `prompt_eval_count` is missing;
- evaluated + 16 >= `num_ctx`;
- `done_reason` is "length".

Rejected answers are never cached. A repair prompt that fails preflight ends that window's repair chain.
The old admission "the hosted Pi path has no checks" no longer applies, because the Pi path is gone.
(admit) The estimate is a heuristic. The worker model's keep-alive is fixed at 1 h and ignores `OLLAMA_KEEP_ALIVE`.

**22. What guarantees one extraction per namespace at a time?**
Where: `knowledge_store/jobs.rs:25-120`, `migrations/0010_namespace_worker_leases.sql:2-7`.
Answer: The real exclusion is `INSERT … ON CONFLICT(namespace) … WHERE lease_until <= now()` into `memory_namespace_leases`. `SELECT … FOR UPDATE SKIP LOCKED` with `NOT EXISTS` is only a first filter. An earlier pending, running or failed extract job blocks later ones (59-67).
Test: `parallel_claims_serialize_a_bank_and_preserve_history_order`.

**23. How does a job keep its lease during a long model call, and what stops a stale worker?**
Where: `worker/mod.rs:119-143`, `assertions.rs:22-43`.
Answer: Every 30 s a heartbeat renews both the job lease and the namespace lease by 15 minutes. If either lease is lost, the work is cancelled. The final write re-checks ownership under the advisory lock. One heartbeat covers every window of the job.
(admit) A database error in the heartbeat cancels a healthy call. A narrow window remains in which a stale transaction can still commit. A job with many windows holds its lease longer, which delays recovery if the worker dies. The heartbeat itself has no test.

**24. What happens when the model call fails?**
Where: `jobs.rs:114` and 122-160 (backoff at 140), `max_attempts` at `0003:74`, retry at `v2/mod.rs:148-156` and `jobs.rs:162-168`.
Answer: Attempts increment when a job is claimed, up to a maximum of 5. The backoff is 2, 4, 8 and 16 s. After the fifth failure the job is marked failed. Recovery is `POST /api/v2/jobs/{id}/retry`, which resets the attempts and reuses the frozen snapshot. Windows that already passed replay from the cache, so a retry is cheap.
(admit) One poison episode blocks every later extract in its namespace; the graph page shows them as "blocked". One failing window fails the whole job, and nothing from the earlier windows is stored.

**25. The model returns JSON that parses but is wrong. What happens?**
Where: `worker/extraction.rs:93-168` (`extract_window`), `knowledge.rs:153-266`.
Answer: Each window gets one try plus 2 repairs. Each reply goes through the schema check, then `validate_claim`, then a check that every related id is in the window's facts. The repair prompt is the original prompt, the previous reply and the error. If all three fail, or a call errors mid-chain, the cached replies are forgotten and the job fails. `apply_extraction` validates again over the whole episode. A job can make up to 3 × windows model calls.
(admit) The previous reply can be up to `num_predict` tokens on top of a 1,536-token reserve, so preflight refusal of a repair is realistic. No end-to-end test covers a successful repair.

**26. Where is the model cache, and what is not in its key?**
Where: `model.rs:167-169` and 184-222, versions at `worker/mod.rs:26-27`.
Answer: The key is the SHA-256 of `ollama/<model>/ctx<num_ctx>`, the version and the prompt. `num_ctx` is now in the key.
(admit) The key leaves out `num_predict`, the system prompt (`worker_system.txt`) and the model's weights or digest. Re-pulling a new `memex-extractor` under the same tag replays old answers unless `EXTRACT_VERSION` is bumped. There is no single-flight, so two identical concurrent prompts both call the model, and there is no eviction.

## E. Graph and clear

**27. How is a late projection job stopped from resurrecting a cleared fact?**
Where: `migrations/0004_projection_fencing.sql`, `worker/projection.rs:13-29`, `graph.rs:60-79` (gate at 63), `knowledge_store/maintenance.rs:81-83`, `recall.rs:583-595`.
Answer: Every insert or update takes a new value from `nextval('assertion_revision_sequence')`. The Cypher MERGEs a gate node and proceeds only if `$revision >= gate.min_revision`. Clear takes its `min_revision` from the same sequence inside its transaction. Every Postgres writer to a namespace takes the same advisory lock, which orders revisions against clear. Recall also re-checks every Neo4j id in Postgres.
Test: `evidence_lifecycle_and_projection` tests the gate directly, including clear at 201 dropping revision 200.
(admit) A dropped stale write looks like success, with no log or counter. No test races a real queued job against clear, or one clear against another.

**28. Walk me through the clear: confirmation, delete order, :legacy, traces.**
Where: `src/v2/graph.rs:39-73`, `knowledge_store/maintenance.rs:4-98` (`STEPS` at 23-68, scope at 71), `scripts/clear-graph.sh`.
Answer: `confirm` must repeat the namespace exactly, otherwise the request gets a 400. Then one transaction does the following:
1. takes the lock;
2. returns 409 if a job holds a live lease;
3. runs 14 statements over `[ns, ns:legacy]`, children first;
4. takes `nextval`;
5. enqueues a `clear_graph` job;
6. commits.

The handler then clears Neo4j inline. Traces in `operations` are kept but unlinked. Any foreign-key error rolls back everything.

Nothing writes V1 tables any more. The legacy steps and the `:legacy` scope remain only to clean databases upgraded from older releases.
Test: `clearing_the_graph_removes_one_namespace_completely_and_leaves_the_others`.
(admit) The delete list is hard-coded. Neo4j clear does not remove V1-era `:Entity`, `:Claim` or `:MemoryVersion` nodes. The running-job check ignores the `:legacy` scope.

**29. A job is running when someone presses Clear. Where are the holes?**
Where: `maintenance.rs:9-20`, `jobs.rs:25-120`, `migrations/0009_legacy_adapter.sql:11`.
Answer: The advisory lock plus a count of jobs with `lease_until > now()` gives 409. An expired lease does not count.
(admit) `claim_job` takes no advisory lock, so a claim can commit between the count and the delete, and that is untested. The later write is fenced: by the revision gate for project jobs, and by the lease check for extract jobs. Pending jobs, including an older `clear_graph` job, are deleted silently. The V1 trigger's different lock key (`hashtext` rather than `hashtextextended`) is still installed, but no API path reaches it now.

**30. Explain the Neo4j clear Cypher.**
Where: `graph.rs:82-99`, `v2/graph.rs:58-69`, `worker/projection.rs:39-52`.
Answer: The Cypher does four things:
1. MERGEs the gate node;
2. raises `min_revision` with a CASE, so it never goes down;
3. DETACH DELETEs the assertions below it;
4. deletes entities left with no assertion.

Running it twice is safe, so the inline call and the queued job can both run. If Neo4j is down, the response is 200 with `graph.cleared=false` and a reason, and the page shows an alert.
(admit) `projection.rs:47` uses `unwrap_or(0)`, so a malformed payload turns into a no-op clear reported as success. Gate nodes are never deleted. No test covers the Neo4j-down path.

**31. How does the service talk to Neo4j, and what bounds the expansion?**
Where: `graph.rs:100-195` and 252-291, `recall.rs:539-632`.
Answer: It uses the HTTP transaction endpoint (`/db/{db}/tx/commit`) with basic auth and a 15 s timeout, not Bolt. Errors can arrive inside a 200 body, and the code checks that array. Expansion runs 2 rounds from at most 20 seeds, with LIMIT 20 per stage, 100 rows per round and a 200-row cap. A Neo4j failure degrades recall rather than failing it.
(admit) Neighbours are ordered by id, not by relevance. The graph page's `projection()` read uses three separate transactions, so its counts can disagree under concurrent writes.

**32. How does the graph page work?**
Where: `frontend/src/graph/GraphPage.tsx`, `viewer.js` (poll at 39-54, `reconcile` at 252, `cycle` at 1636-1667, `pollLoop` at 1682, `clearNamespace` at 1980-2019), `layout.js` and `data.js`. Served by `src/web.rs`.
Answer: React and react-bootstrap render only the controls. The graph itself is imperative JavaScript drawing on one `<canvas>`, with a hand-written force layout using Barnes-Hut and no graph library.
- **Polling.** The page polls `/api/v2/graph/projection` (Neo4j) and `/api/v2/graph/memories` (Postgres) every 2 s (`?poll=`, with a 500 ms floor). It pauses while the tab is hidden and sleeps after each cycle, so polls never overlap each other.
- **Redraws.** If a response's text is unchanged, the redraw is skipped. Nodes are diffed, so removed ones fade out.
- **Read-only mode.** `?readonly=1` hides Clear, and the demo iframe uses it.

(admit) Clear calls `cycle()` while the poll loop may also be running. Responses carry no sequence number, so a slow pre-clear response can redraw the old graph until the next poll. The two endpoints are not one snapshot, and memories citing unprojected facts show up as `danglingSupports`. No test covers polling or the clear flow.

## F. Pi extension

**33. Name the five hooks and what each skips.**
Where: `integrations/pi/extension.ts`. The hooks are `session_start` (81-91), `before_agent_start` (93-119), `tool_result` (120-157), `agent_settled` (248-251) and `session_shutdown` (252-258). Two tools and three commands follow (259-388).
Answer:
- `session_start` rebuilds the map of captured tool calls from the branch.
- `before_agent_start` skips an empty prompt. Otherwise it recalls and injects the result.
- `tool_result` skips `memory_*` tools and calls it has already captured. It caps content at 100,000 characters (setting `truncated`) and spools each result at once under `tool:<session>:<callId>`.
- `agent_settled` retains the new messages.
- `session_shutdown` waits for the final flush.

(admit) `flushing = flushing.then(settle)`: if settle rejects once, every later settle, the final flush, `/memory-namespace` and `/memory-clear` all fail. A spool write error escapes `tool_result`.

**34. How is a spooled batch made durable?**
Where: `integrations/pi/spool.ts:17-39` (enqueue), 41-57 (`discardNamespace`) and 58-81 (flush); location at `extension.ts:74-80`.
Answer: Enqueue works in these steps:
1. Create the directory with mode 0700.
2. Name the file with a digest of `[namespace, external_id]`.
3. Open a temp file with `wx` (0600), write it and fsync it.
4. Rename it into place (atomic), then fsync the directory.

Flush reads only `.json` files and deletes each one after a successful POST. The same key overwrites the same file. The spool is now one per namespace, `~/.sdp-memory/<digest>`.
(admit) A failed write leaves a `.tmp` file that is never cleaned. Two sessions in one namespace share a spool, so concurrent flushes can deliver a file twice (harmless, because retain is idempotent) and then hit ENOENT on the unlink. Delivery follows filename order, not time order. Nothing tests `discardNamespace`.

**35. How are only new messages sent, and what can that lose?**
Where: `extension.ts:158-247` (cursor at 164-176, `slice(-1000)` at 229, enqueue then cursor at 232-238).
Answer: An `sdp-memory-cursor` session entry stores `{namespace, upTo}`. The batch is spooled first and then the cursor moves, so a crash resends under the same key and the server accepts it idempotently. Messages from memory tools and assistant messages that ended in an error are skipped.
(admit) `events.slice(-1000)` drops the oldest events of a larger backlog, and the cursor still moves past them. A cursor that is not on the current branch resends the whole branch under a new key. After `/memory-namespace`, the whole branch is retained again into the new namespace.

**36. How does extraction avoid retaining its own prompts?**
Where: `src/main.rs:39-54` and 94-98; `extension.ts:23`.
Answer: The worker no longer starts Pi at all. It is an HTTP call from Rust to Ollama, so the extension never sees an extraction prompt. The `MEMORY_WORKER === "1"` guard remains as a safety net for the Pi processes that `benchmark/run.py` (reader and judge) and `check-worker.py` start, and both of those also pass `--no-extensions`.
(admit) The guard matches only the exact string "1", and no test covers it.
Trap: "So what stops a feedback loop now?" Injected recall context has its own `customType` and is excluded from retained batches, and the extension test asserts this.

**37. Where does 0.45 come from?**
Where: `integrations/pi/client.ts:8-21` (constant and notes), 65-68 and 84-89; `extension.ts:101`; `docs/evidence-memory.md:100-140`.
Answer: It was calibrated on one namespace (`user:dev`, about 320 facts) with `qwen3-embedding:0.6b`. Relevant facts fell between 0.236 and 0.439, and the best unrelated hits between 0.445 and 0.564. On 80 prompts:

| Cutoff | Answerable prompts with the answer in context (of 55) | No-answer prompts with more than 1,000 chars injected (of 25) |
|---|---|---|
| none | 39 | 25 |
| 0.42 | 50 | 1 |
| 0.45 | 49 | 6 |

The server has no default value.
(admit) The margin is thin: a relevant fact was measured at 0.465. Short prompts are not separated ("ok" scores 0.415). 0.42 was stricter, but 0.45 was kept. The constant ignores `EMBEDDING_MODEL`. An older service ignores the field, and the client only warns.

**38. How does the scripted end-to-end check prove cross-directory recall?**
Where: `scripts/pi-rpc-e2e.py` (sessions at 72-95, setup at 293-307, scenarios at 318-456, exit at 474-478).
Answer: Each session is its own `pi --mode rpc` process with its own session directory and working directory (A to E, only A a git repo). Built-in tools are off, so the model cannot read files. The namespace is `e2e:<run>`, with a private spool and a random codename `Marmalade-<hex>`. The scenarios:
1. Session A states the codename and "Fridays".
2. Session B, before extraction, must show an injected memory message and answer with the codename and "friday".
3. A control in C without the extension must not know it.
4. After `/api/v2/status` reports ready with no failed jobs, C must answer, and `/api/v2/graph` must contain the fact.
5. A correction in D to "Tuesdays" must make E answer Tuesday.

(admit) The checks are substring matches on model text. The script needs a paid provider and a running service, is not in `scripts/test.sh`, and has no committed run. With `--realistic`, a globally installed extension would also load in the control session.

## G. New model, evidence and operations

**39. How is a long session split into windows, and how do claims keep correct evidence?**
Where: `worker/window.rs:44-55` (`shorten_content`), 59-100 (`compact_event`), 104-120 (`windows`); `worker/extraction.rs:47-72`.
Answer: Each event is shortened to at most 1,600 characters (the first 1,100 and the last 360), and ids, commit hashes and file bodies are removed from tool metadata. Windows are cut greedily at about 2,800 estimated tokens, with at least one event each. Indices are numbered from 0 inside a window and shifted by `range.start` after validation. Quotes are checked against the full event.
Tests: `a_long_session_is_read_in_windows_and_claims_keep_episode_indices`, `windows_cover_every_event_once_and_in_order`, `one_oversized_event_gets_its_own_window`.
(admit) A claim cannot cite events in two different windows. A correction that spans windows becomes contested. A fact that only appears in the omitted middle of a long output is invisible to the model.

**40. How do you keep the prompt you serve the same as the prompt the model was trained on?**
Where:
- Templates: `src/worker/extract_prompt.txt`, `consolidate_prompt.txt`, `worker_system.txt`, `src/v2/reflect_prompt.txt`.
- Filler: `prompts.rs:18-39` (`fill_template`).
- Shared fixture: `tests/fixtures/extract_prompt_parity.json`.
- Python side: `slm-distill/prompt.py`, `make_parity_fixture.py`, `test_prompt_parity.py`.
- Rust tests: `prompts.rs:187`, `window.rs:255`, `consolidation.rs:147`, `reflect.rs:93`.

Answer: Both sides read the same template files. `fill_template` fills placeholders in one pass, so a stored fact containing `{{EVENTS}}` cannot change the prompt. One fixture holds the rendered prompts for extraction, windowing, consolidation and reflection. Python regenerates it and compares; Rust renders the same inputs and compares; if either side drifts, one test fails.
(admit) `scripts/test.sh` does not run the Python half. Python's `fit_existing` drops the oldest facts first, unlike Rust's mention ranking, and the fixture does not cover that path. The system prompt is shared by file but is not in the fixture.

**41. What does slm-distill train, and on what?**
Where: `slm-distill/scenarios.py`, `teacher.py`, `label.py`, `rules.py`, `aux_tasks.py`, `build_dataset.py`, `train.py`, `export.py`; earlier experiment in `slm-distill/v1/`.
Answer:
1. **Data.** A teacher model, used offline only and cached on disk, writes synthetic Pi coding-session timelines across about 30 stacks. Each timeline has planted facts, preferences, corrections and dated events, plus traps: secrets, guesses, prompt injection and noise.
2. **Labels.** `label.py` labels windows the way the engine would. A label is kept only if `rules.py`, a Python port of the Rust acceptance rules, accepts it after the same repair loop. `aux_tasks.py` adds consolidation and reflection examples, so one model serves all three jobs.
3. **Splits.** The data is split by timeline: 0-259 train, 260-279 validation, 280-319 test.
4. **Training.** `train.py` trains a LoRA (r=64) on Qwen3-1.7B with loss on the answer only.
5. **Export.** `export.py` merges the weights, converts them to GGUF and runs `ollama create memex-extractor`.

(admit) No results for this pipeline are in the repo. The only metrics are v1's, on a different schema (relaxed F1 0.398). All the data and labels are synthetic. `scripts/fetch-slm.sh` is referenced but missing.
Trap: "Does the teacher run at serve time?" No.

**42. What does the benchmark compare, and has it been run?**
Where: `benchmark/run.py` (conditions at 352-356, reader and judge at 196-251, worker check at 253-263, embedding-gap gate at 293-295), `benchmark/config.json`, `benchmark/README.md`.
Answer: The dataset is LongMemEval-S, 500 questions with a pinned checksum. There are three primary conditions:
- `raw_hybrid`: raw text only, no graph, no time channel;
- `enhanced`: everything on;
- `full_history`: no retrieval.

Two ablations follow: `no_graph` (`graph:false`) and `no_consolidation` (`observations:false`). Results are compared with paired bootstrap intervals and a blinded human audit packet.
(admit) Nothing is complete. The only partial run has 102 full-history answers and 90 judgments. No ablation has been run, and no improvement is claimed. As written, the runner refuses to start against the Ollama worker, because it expects `pi/…` as the worker identity.
Trap: "Is the ablation 100 or 500 questions?" Retrieval runs on all 500; answers use a fixed 100-question sample.

**43. What is validated at startup, and what stops it?**
Where: `src/main.rs:31-140`, `model.rs:49-100`, `recall.rs:824-838`.
Answer: Startup stops when:
- `MEMORY_MODEL_PROVIDER` is anything other than `ollama`;
- `OLLAMA_NUM_CTX` or `OLLAMA_NUM_PREDICT` is invalid, or predict >= ctx;
- `TEMPORAL_PLANNER` is not one of `rules`, `model` or `off`;
- the database connection or a migration fails;
- `BIND_ADDR` is unusable, or the bind fails.

These only log:
- Neo4j constraints not ready (an empty `NEO4J_URI` disables the graph);
- the worker model probe (`/api/show`) failing;
- the embedder warm-up failing.

(admit) An invalid `MEMORY_WORKER_CONCURRENCY` silently becomes 4 (clamped to 1-8). Leftover `PI_PROVIDER`/`PI_MODEL` are silently ignored. The service accepts retains even when the worker model is missing. `/healthz` does not say whether the model is served.
Trap: "Does `MEMORY_MODEL_PROVIDER=ollama` change anything?" No, it is accepted as a no-op.

**44. What do the run and test scripts do?**
Where: `scripts/run-memory.sh`, `scripts/test.sh`.
Answer: `run-memory.sh` builds the frontend, starts Postgres and Neo4j, creates `memory_app`, starts Ollama if needed, pulls the embedder, builds the release binary (host cargo if it matches the pinned toolchain, otherwise Docker) and runs it. It builds release because debug was about twice as slow on large recalls.

`test.sh` runs these in order:
1. the frontend build and vitest;
2. starts the databases and creates `memory_test`;
3. `cargo fmt --check`, `clippy -D warnings`, and `cargo test --all-targets` with `--test-threads=1`, because the tests share one Postgres and one Neo4j;
4. the benchmark unittests;
5. the Pi `tsc` check and `npm test`.

(admit) `run-memory.sh` still requires Pi and probes a Pi model the service no longer uses, and it never fetches `memex-extractor`. `test.sh` skips Playwright and the slm-distill tests.

**45. How is the hosted setup deployed?**
Where: `deploy/azure/` (docker-compose.yml, Caddyfile.template, provision.sh, deploy.sh, run-local.sh, pi-session.sh), `frontend/src/demo/`.
Answer: Compose runs these services:
- Postgres with pgvector;
- Neo4j;
- Ollama, with an init step that pulls the embedder;
- the engine;
- ttyd running one fresh Pi per browser connection, in one of three project directories;
- Caddy, with an access code. Only graph reads are exposed, and `/` redirects to `/demo`.

The demo page puts the terminal and a read-only graph side by side. `deploy.sh` swaps the image and rolls back if readiness fails. `run-local.sh` runs the same stack locally.
(admit) Provisioning has never been executed. The compose file still configures a Pi worker and serves no `memex-extractor`, so extraction would fail as configured. The namespace comes from the URL, and every hosted user is `user:dev`.
Trap: "Can a visitor clear memory from the graph page?" No. Clear is hidden and not proxied. It goes through Pi's `/memory-clear`, which requires typed confirmation.

**46. How many tests are there?**
Counted with grep at 1797536, not run here.

| Suite | Tests |
|---|---|
| Rust integration, `tests/evidence_graph.rs` | 28 |
| Rust integration, `tests/ollama_context.rs` | 13 (3 `live_*` need `OLLAMA_LIVE_TEST=1`) |
| Rust unit | 55 |
| Python, benchmark | 5 |
| Python, slm-distill | 4 |
| Pi extension | 9 |
| Frontend, vitest | 4 |
| Frontend, Playwright | 9 |

Rust unit tests by file: model 10, prompts 14, window 11, knowledge 7, temporal 6, graph 4, consolidation 1, recall 1, reflect 1.
(admit) Without `TEST_DATABASE_URL`, the integration tests print "skipped" and pass. No end-to-end test runs in CI. Nothing tests the `MEMORY_WORKER` guard, `/memory-namespace` or `/memory-clear`.

**47. What observability is there?**
Where: `src/api.rs:27-30` and 59-114, `src/observability.rs`, `src/v2/mod.rs:175-215`, `main.rs:19-30` and 152-159.
Answer:
- **`/metrics`** is built on every scrape from the durable `operations` table, so the counts survive restarts.
- **Traces.** Every retain, recall, reflect and worker step writes a trace with ordered steps, readable at `/api/v2/traces`. For example, `graph_expansion` lists `dropped_by_max_distance`.
- **`/api/v2/status`** reports job groups, `outstanding_jobs`, `ready`, stale observations, the last projection time, worker timings and `embedding_gaps`. The benchmark refuses to run while gaps exist.
- **`/healthz`** reports the database, the embedder, `worker_model`, `num_ctx`/`num_predict`, concurrency and the embedding model.
- **Logs** are JSON.

(admit) `ready` ignores `embedding_gaps`. `/metrics` scans the whole table, mixes all namespaces and has no authentication.
Trap: "Does `ready: true` mean recall is good?" No. Embedding gaps and projection lag are reported separately.
