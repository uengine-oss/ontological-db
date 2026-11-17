-- Run as robo (owner of robo_t1): CHECK constraints, row/statement triggers,
-- column defaults and generated columns under Cypher writes.
\pset footer off
SET search_path = ag_catalog, "$user", public;

\echo '### setup: a probe vertex label, a log for trigger calls, row + statement triggers, two CHECKs'
SELECT ag_catalog.create_vlabel('robo_t1', 'TrgProbe');
CREATE SCHEMA IF NOT EXISTS robo_aux;
CREATE TABLE robo_aux.trg_log (id bigserial PRIMARY KEY, tg text, lvl text, op text, at timestamptz DEFAULT now());
CREATE FUNCTION robo_aux.log_trg() RETURNS trigger LANGUAGE plpgsql AS $f$
BEGIN
  INSERT INTO robo_aux.trg_log (tg, lvl, op) VALUES (TG_NAME, TG_LEVEL, TG_OP);
  IF TG_LEVEL = 'ROW' THEN RETURN coalesce(NEW, OLD); END IF;
  RETURN NULL;
END $f$;
CREATE TRIGGER t_before_row BEFORE INSERT OR UPDATE OR DELETE ON robo_t1."TrgProbe"
  FOR EACH ROW EXECUTE FUNCTION robo_aux.log_trg();
CREATE TRIGGER t_after_row AFTER INSERT OR UPDATE OR DELETE ON robo_t1."TrgProbe"
  FOR EACH ROW EXECUTE FUNCTION robo_aux.log_trg();
CREATE TRIGGER t_after_stmt AFTER INSERT OR UPDATE OR DELETE ON robo_t1."TrgProbe"
  FOR EACH STATEMENT EXECUTE FUNCTION robo_aux.log_trg();

ALTER TABLE robo_t1."TrgProbe"
  ADD CONSTRAINT agg_name_required
  CHECK ((properties OPERATOR(ag_catalog.->>) 'name'::text) IS NOT NULL);
ALTER TABLE robo_t1."TrgProbe"
  ADD CONSTRAINT agg_version_nonneg
  CHECK ((properties OPERATOR(ag_catalog.->>) 'version'::text) IS NULL
         OR (properties OPERATOR(ag_catalog.->>) 'version'::text)::int >= 0);
SELECT conname, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = 'robo_t1."TrgProbe"'::regclass AND contype = 'c';

\echo '### T1 Cypher writes that satisfy the CHECKs'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (a:TrgProbe {name: 'Trg1', version: 1}) RETURN a.name $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe {name: 'Trg1'}) SET a.version = 2 RETURN a.version $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MERGE (a:TrgProbe {name: 'Trg2'}) RETURN a.name $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe {name: 'Trg2'}) DELETE a $$) AS (x ag_catalog.agtype);
\echo '-- trigger calls caused by those four Cypher statements'
SELECT tg, lvl, op, count(*) FROM robo_aux.trg_log GROUP BY 1, 2, 3 ORDER BY 1, 2, 3;
SELECT count(*) AS trigger_calls_after_cypher FROM robo_aux.trg_log;

\echo '### T3 Cypher writes that break a CHECK'
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (a:TrgProbe {bc: 'no name'}) RETURN a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (a:TrgProbe {name: 'Neg', version: -1}) RETURN a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe {name: 'Trg1'}) SET a.version = -5 RETURN a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe {name: 'Trg1'}) REMOVE a.name RETURN a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe {name: 'Trg1'}) SET a = {bc: 'replaced'} RETURN a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MERGE (a:TrgProbe {name: 'MergeNeg', version: -3}) RETURN a $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe) WHERE a.name STARTS WITH 'Neg' OR a.name = 'MergeNeg' RETURN count(a) $$) AS (x ag_catalog.agtype);
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MATCH (a:TrgProbe {name: 'Trg1'}) RETURN a.version $$) AS (x ag_catalog.agtype);

\echo '### T2 an extra column with a default and a generated column, then Cypher CREATE'
ALTER TABLE robo_t1."TrgProbe" ADD COLUMN note text DEFAULT 'dflt';
ALTER TABLE robo_t1."TrgProbe" ADD COLUMN name_gen text
  GENERATED ALWAYS AS (properties OPERATOR(ag_catalog.->>) 'name'::text) STORED;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ CREATE (a:TrgProbe {name: 'Trg3'}) RETURN a $$) AS (x ag_catalog.agtype);
SELECT properties::text, note, name_gen FROM robo_t1."TrgProbe" ORDER BY id;

\echo '### T4 control: the same table through SQL fires the triggers'
TRUNCATE robo_aux.trg_log;
INSERT INTO robo_t1."TrgProbe" (properties) VALUES ('{"name": "SqlRow", "version": 0}'::ag_catalog.agtype);
UPDATE robo_t1."TrgProbe" SET properties = '{"name": "SqlRow", "version": 1}'::ag_catalog.agtype WHERE name_gen = 'SqlRow';
DELETE FROM robo_t1."TrgProbe" WHERE name_gen = 'SqlRow';
SELECT tg, lvl, op, count(*) FROM robo_aux.trg_log GROUP BY 1, 2, 3 ORDER BY 1, 2, 3;
INSERT INTO robo_t1."TrgProbe" (properties) VALUES ('{"bc": "sql without name"}'::ag_catalog.agtype);

\echo '### T5 AGE''s own statement trigger _age_cache_invalidate exists on every label table'
SELECT tgname, tgfoid::regproc, tgtype FROM pg_trigger WHERE tgrelid = 'robo_t1."TrgProbe"'::regclass ORDER BY 1;

\echo '### T6 label tables and extra columns: MERGE decides vertex vs edge by the column count'
SELECT count(*) AS natts FROM pg_attribute WHERE attrelid = 'robo_t1."TrgProbe"'::regclass AND attnum > 0 AND NOT attisdropped;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MERGE (a:TrgProbe {name: 'Merge4'}) RETURN a.name $$) AS (x ag_catalog.agtype);
ALTER TABLE robo_t1."TrgProbe" ADD COLUMN third_extra int;
SELECT count(*) AS natts FROM pg_attribute WHERE attrelid = 'robo_t1."TrgProbe"'::regclass AND attnum > 0 AND NOT attisdropped;
SELECT * FROM ag_catalog.cypher('robo_t1', $$ MERGE (a:TrgProbe {name: 'Merge5'}) RETURN a.name $$) AS (x ag_catalog.agtype);
\echo '-- DROP COLUMN on a label table is refused by AGE (its drop hook sees the label table)'
ALTER TABLE robo_t1."TrgProbe" DROP COLUMN third_extra;

\echo '### cleanup'
DROP TRIGGER t_before_row ON robo_t1."TrgProbe";
DROP TRIGGER t_after_row ON robo_t1."TrgProbe";
DROP TRIGGER t_after_stmt ON robo_t1."TrgProbe";
ALTER TABLE robo_t1."TrgProbe" DROP CONSTRAINT agg_name_required, DROP CONSTRAINT agg_version_nonneg;
SELECT ag_catalog.drop_label('robo_t1', 'TrgProbe');
DROP SCHEMA robo_aux CASCADE;
