CREATE SEQUENCE assertion_revision_sequence;
ALTER TABLE assertions ADD COLUMN revision bigint NOT NULL DEFAULT nextval('assertion_revision_sequence');
CREATE FUNCTION advance_assertion_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN NEW.revision := nextval('assertion_revision_sequence'); RETURN NEW; END $$;
CREATE TRIGGER assertions_revision BEFORE UPDATE ON assertions FOR EACH ROW EXECUTE FUNCTION advance_assertion_revision();
