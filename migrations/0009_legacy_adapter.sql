-- Bring migrated slots into the same resolved-entity identity used by V2.
UPDATE assertions SET slot_key=subject_id::text || '::' || lower(regexp_replace(trim(predicate),'\s+',' ','g'))
WHERE id IN (SELECT id FROM memory_versions);

CREATE FUNCTION sync_legacy_assertion(version_id uuid) RETURNS void LANGUAGE plpgsql AS $$
DECLARE v record; entity_id uuid; projected_revision bigint; old_id uuid; changed_ids uuid[] := ARRAY[]::uuid[];
BEGIN
 SELECT mv.*,m.namespace,m.subject,m.predicate INTO v FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id WHERE mv.id=version_id;
 -- Benchmark shadow banks intentionally retain the unmodified V1 representation.
 IF NOT FOUND OR right(v.namespace,7)=':legacy' THEN RETURN; END IF;
 PERFORM pg_advisory_xact_lock(hashtext(v.namespace));
 INSERT INTO entities(namespace,entity_type,name,normalized_name)
 VALUES(v.namespace,'person',v.subject,lower(regexp_replace(trim(v.subject),'\s+',' ','g')))
 ON CONFLICT(namespace,entity_type,normalized_name) DO UPDATE SET name=entities.name RETURNING id INTO entity_id;
 IF v.status='active' THEN
  FOR old_id IN SELECT id FROM assertions WHERE namespace=v.namespace AND slot_key=entity_id::text || '::' || lower(regexp_replace(trim(v.predicate),'\s+',' ','g')) AND cardinality='single' AND status='active' AND id<>version_id FOR UPDATE LOOP
   UPDATE assertions SET status='superseded',valid_to=v.valid_from WHERE id=old_id;
   changed_ids := array_append(changed_ids,old_id);
   INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload) SELECT namespace,'project','legacy-adapter:'||id||':'||revision,jsonb_build_object('assertion_id',id) FROM assertions WHERE id=old_id ON CONFLICT DO NOTHING;
  END LOOP;
 END IF;
 INSERT INTO assertions(id,namespace,subject_id,predicate,value,normalized_value,statement,slot_key,cardinality,kind,status,valid_from,valid_to,recorded_at,confidence,model,embedding)
 VALUES(v.id,v.namespace,entity_id,v.predicate,v.value,v.normalized_value,v.statement,entity_id::text||'::'||lower(regexp_replace(trim(v.predicate),'\s+',' ','g')),'single',v.kind,v.status,v.valid_from,v.valid_to,v.created_at,1,v.extractor_version,v.embedding)
 ON CONFLICT(id) DO UPDATE SET status=excluded.status,valid_to=excluded.valid_to,statement=excluded.statement,embedding=excluded.embedding
 RETURNING revision INTO projected_revision;
 FOREACH old_id IN ARRAY changed_ids LOOP
  INSERT INTO assertion_edges(namespace,from_id,to_id,relation,explanation) VALUES(v.namespace,version_id,old_id,'supersedes','legacy resolver update') ON CONFLICT DO NOTHING;
 END LOOP;
 INSERT INTO assertion_sources SELECT version_id,s.chunk_id,c.content FROM memory_version_sources s JOIN chunks c ON c.id=s.chunk_id WHERE s.memory_version_id=version_id ON CONFLICT DO NOTHING;
 INSERT INTO assertion_entities VALUES(v.namespace,version_id,entity_id) ON CONFLICT DO NOTHING;
 INSERT INTO assertion_edges(namespace,from_id,to_id,relation,explanation)
 SELECT v.namespace,r.from_version_id,r.to_version_id,r.relation_type,'legacy resolver relation' FROM memory_relations r WHERE r.from_version_id=version_id AND EXISTS(SELECT 1 FROM assertions WHERE id=r.to_version_id AND namespace=v.namespace) ON CONFLICT DO NOTHING;
 IF v.status<>'active' THEN changed_ids := array_append(changed_ids,version_id); END IF;
 IF cardinality(changed_ids)>0 THEN
  WITH RECURSIVE dependents(id) AS (
   SELECT from_id FROM assertion_edges WHERE namespace=v.namespace AND to_id=ANY(changed_ids) AND relation IN ('supports','derives')
   UNION SELECT e.from_id FROM assertion_edges e JOIN dependents d ON e.to_id=d.id WHERE e.namespace=v.namespace AND e.relation IN ('supports','derives')
  ), changed AS (UPDATE assertions SET status='stale' WHERE namespace=v.namespace AND id IN (SELECT id FROM dependents) AND kind='observation' AND status IN ('active','contested') RETURNING id,revision)
  INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload) SELECT v.namespace,'project','legacy-adapter:'||id||':'||revision,jsonb_build_object('assertion_id',id) FROM changed ON CONFLICT DO NOTHING;
  INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload) VALUES(v.namespace,'consolidate','legacy-change:'||version_id||':'||projected_revision,jsonb_build_object('changed_assertion_id',version_id)) ON CONFLICT DO NOTHING;
 END IF;
 INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload) VALUES(v.namespace,'project','legacy-adapter:'||version_id||':'||projected_revision,jsonb_build_object('assertion_id',version_id)) ON CONFLICT DO NOTHING;
END $$;
CREATE FUNCTION sync_legacy_assertion_trigger() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_TABLE_NAME='memory_versions' THEN PERFORM sync_legacy_assertion(NEW.id);
 ELSIF TG_TABLE_NAME='memory_version_sources' THEN PERFORM sync_legacy_assertion(NEW.memory_version_id);
 ELSE PERFORM sync_legacy_assertion(NEW.from_version_id);
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER legacy_assertion_version AFTER INSERT OR UPDATE ON memory_versions FOR EACH ROW EXECUTE FUNCTION sync_legacy_assertion_trigger();
CREATE TRIGGER legacy_assertion_source AFTER INSERT ON memory_version_sources FOR EACH ROW EXECUTE FUNCTION sync_legacy_assertion_trigger();
CREATE TRIGGER legacy_assertion_relation AFTER INSERT ON memory_relations FOR EACH ROW EXECUTE FUNCTION sync_legacy_assertion_trigger();
