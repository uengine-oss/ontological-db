-- Releases, schema diff and timelines (engine/sql/release.sql).
-- A schema evolved in one graph ("dev") is diffed against another ("ops"),
-- applied as ordinary schema changes inside a release, and a clause amended
-- and corrected over time is read back as a timeline.
\set ON_ERROR_STOP on
SET client_min_messages = warning;

SELECT og_create_graph('dev') \g /dev/null
SELECT og_create_graph('ops') \g /dev/null

-- dev: v1 → evolve (rename + retype)
SELECT og_create_type('dev', 'Clause', 'entity') \g /dev/null
SELECT og_add_property('dev', 'Clause', 'clause_key', 'string') \g /dev/null
SELECT og_add_property('dev', 'Clause', 'article', 'string') \g /dev/null
SELECT og_add_property('dev', 'Clause', 'text', 'string') \g /dev/null
SELECT og_enable_temporal('dev', 'Clause') \g /dev/null
SELECT og_create_type('dev', 'Product', 'entity') \g /dev/null
SELECT og_add_property('dev', 'Product', 'code', 'string') \g /dev/null
SELECT og_add_property('dev', 'Product', 'max_loss_pct', 'string') \g /dev/null
SELECT og_create_type('dev', 'ELS', 'entity', ARRAY['Product']) \g /dev/null
SELECT og_schema_apply('dev', '{"kind":"rename_property","type":"Clause","property":"article","new_name":"article_no"}') \g /dev/null
SELECT og_schema_apply('dev', '{"kind":"alter_property_type","type":"Product","property":"max_loss_pct","to":"numeric"}') \g /dev/null

-- ops already has v1 of Clause (with the old name)
SELECT og_create_type('ops', 'Clause', 'entity') \g /dev/null
SELECT og_add_property('ops', 'Clause', 'clause_key', 'string') \g /dev/null
SELECT og_add_property('ops', 'Clause', 'article', 'string') \g /dev/null
SELECT og_add_property('ops', 'Clause', 'text', 'string') \g /dev/null

\echo == diff ops -> dev
SELECT jsonb_pretty(og_schema_diff('ops', og_schema_snapshot('dev'),
         (SELECT jsonb_agg(to_jsonb(c)) FROM og_schema_changes('dev') c)));

DO $$
DECLARE d jsonb := og_schema_diff('ops', og_schema_snapshot('dev'),
                     (SELECT jsonb_agg(to_jsonb(c)) FROM og_schema_changes('dev') c));
        kinds text[] := ARRAY(SELECT x->>'kind' FROM jsonb_array_elements(d) x);
BEGIN
    ASSERT kinds[1] = 'rename_property', format('rename first, got %s', kinds);
    ASSERT 'create_type' = ANY(kinds) AND 'enable_temporal' = ANY(kinds), format('got %s', kinds);
    -- ELS inherits code/max_loss_pct: only Product declares them
    ASSERT NOT EXISTS (SELECT 1 FROM jsonb_array_elements(d) x
                        WHERE x->>'type' = 'ELS' AND x->>'kind' = 'add_property'), 'inherited props repeated';
    ASSERT NOT EXISTS (SELECT 1 FROM jsonb_array_elements(d) x WHERE x->>'kind' = 'drop_property'),
           'rename must not look like a drop';
END $$;

-- apply the diff inside a release
DO $$
DECLARE rid int4 := og_release_begin('ops', 'R1', 'dev', 'sha256:abc');
        c jsonb; ids int4[] := '{}'; r jsonb;
BEGIN
    FOR c IN SELECT x FROM jsonb_array_elements(og_schema_diff('ops', og_schema_snapshot('dev'),
                (SELECT jsonb_agg(to_jsonb(sc)) FROM og_schema_changes('dev') sc))) x LOOP
        CASE c->>'kind'
        WHEN 'create_type' THEN
            PERFORM og_create_type('ops', c->>'type', CASE c->>'type_kind' WHEN 'relation' THEN 'relation' ELSE 'entity' END,
                ARRAY(SELECT jsonb_array_elements_text(c->'parents')));
        WHEN 'enable_temporal' THEN PERFORM og_enable_temporal('ops', c->>'type');
        ELSE PERFORM og_schema_apply('ops', c);
        END CASE;
    END LOOP;
    r := og_release_commit(rid, ids, 0, '{}');
    ASSERT r->>'status' = 'applied';
END $$;

\echo == after release, diff is empty
SELECT og_schema_diff('ops', og_schema_snapshot('dev')) AS remaining;
DO $$ BEGIN
    ASSERT jsonb_array_length(og_schema_diff('ops', og_schema_snapshot('dev'))) = 0, 'schemas differ after release';
END $$;

-- a package applied twice is refused
DO $$ BEGIN
    PERFORM og_release_begin('ops', 'R1 again', 'dev', 'sha256:abc');
    RAISE EXCEPTION 'duplicate release was accepted';
EXCEPTION WHEN unique_violation THEN NULL;
END $$;

-- timeline: write, amend (valid 2027-01-01), correct (it was 2027-03-01)
BEGIN; SET LOCAL og.valid_from = '2023-11-16'; SET LOCAL og.source = 'release:1';
SELECT og_cypher('ops', 'CREATE (:Clause {clause_key:"SUPV#9", article_no:"제9조", text:"원금의 100분의 40"})') \g /dev/null
COMMIT;
SELECT pg_sleep(0.02) \g /dev/null
BEGIN; SET LOCAL og.valid_from = '2027-01-01'; SET LOCAL og.source = 'release:2 가상 개정';
SELECT og_cypher('ops', 'MATCH (c:Clause {clause_key:"SUPV#9"}) SET c.text = "원금의 100분의 30"') \g /dev/null
COMMIT;
SELECT now() AS before_fix \gset
SELECT set_config('og_test.before_fix', :'before_fix', false) \g /dev/null
SELECT pg_sleep(0.02) \g /dev/null
-- correction: the 30% text really holds from 2027-03-01. Past valid-time
-- slices are not edited in place; the correction is two ordinary writes —
-- back to 40% from 2027-01-01, then 30% from 2027-03-01 — and the old belief
-- stays in history, readable with system_at.
BEGIN; SET LOCAL og.valid_from = '2027-01-01'; SET LOCAL og.source = 'release:3 정정';
SELECT og_cypher('ops', 'MATCH (c:Clause {clause_key:"SUPV#9"}) SET c.text = "원금의 100분의 40"') \g /dev/null
COMMIT;
SELECT pg_sleep(0.02) \g /dev/null
BEGIN; SET LOCAL og.valid_from = '2027-03-01'; SET LOCAL og.source = 'release:3 정정';
SELECT og_cypher('ops', 'MATCH (c:Clause {clause_key:"SUPV#9"}) SET c.text = "원금의 100분의 30"') \g /dev/null
COMMIT;

\echo == timeline now
SELECT valid_from::date, valid_to::date, props->>'text' AS text, op, source
  FROM og_timeline('ops', 'Clause', 'clause_key', 'SUPV#9', '2026-10-01', '2027-12-31');
\echo == timeline as known before the correction
SELECT valid_from::date, valid_to::date, props->>'text' AS text, op, source
  FROM og_timeline('ops', 'Clause', 'clause_key', 'SUPV#9', '2026-10-01', '2027-12-31', :'before_fix');

DO $$
DECLARE now_t text[] := ARRAY(SELECT props->>'text' FROM og_timeline('ops','Clause','clause_key','SUPV#9','2026-10-01','2027-12-31') ORDER BY valid_from);
BEGIN
    ASSERT now_t = ARRAY['원금의 100분의 40','원금의 100분의 40','원금의 100분의 30'], format('timeline now: %s', now_t);
END $$;
DO $$
DECLARE then_t text[] := ARRAY(SELECT props->>'text' FROM og_timeline('ops','Clause','clause_key','SUPV#9','2026-10-01','2027-12-31',
                          current_setting('og_test.before_fix')::timestamptz) ORDER BY valid_from);
BEGIN
    ASSERT then_t = ARRAY['원금의 100분의 40','원금의 100분의 30'], format('timeline before fix: %s', then_t);
END $$;

\echo == releases
SELECT r->>'name', r->>'status', r->>'from_version', r->>'to_version' FROM og_releases('ops') r;

SELECT og_drop_graph('dev') \g /dev/null
SELECT og_drop_graph('ops') \g /dev/null
\echo 11_release_timeline: reached the end
