-- Schema evolution tutorial — Expand → Migrate → Validate → Contract.
--
-- Companion to docs/tutorials/schema-evolution-tutorial.md. Run it against a
-- fresh database that has the extension installed:
--
--   createdb tut_schema
--   psql -d tut_schema -c 'CREATE EXTENSION ontological CASCADE'
--   psql -d tut_schema -f examples/tutorials/schema_evolution.sql
--
-- Every step prints what it did; the tutorial shows the expected output.
-- Statements that are *meant* to fail are wrapped in DO blocks that print the
-- error instead of stopping the script.
\set ON_ERROR_STOP on
\set QUIET on
\pset footer off
SET client_min_messages = warning;

-- Prints the error a statement raises (or says that it did not fail).
CREATE FUNCTION pg_temp.try(q text) RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    EXECUTE q;
    RETURN 'ok (no error)';
EXCEPTION WHEN others THEN
    RETURN SQLSTATE || ' ' || SQLERRM;
END $$;

\echo
\echo '=== 0. 출발점: age 를 문자열로 잘못 선언한 HR 그래프 ==='
\o /dev/null
SELECT og_create_graph('hr');
SELECT og_create_type('hr', 'Person', 'entity');
SELECT og_add_property('hr', 'Person', 'name', 'string');
SELECT og_add_property('hr', 'Person', 'age',  'string');      -- the mistake
SELECT og_create_type('hr', 'Employee', 'entity', ARRAY['Person']);
SELECT og_create_index('hr', 'Person', 'age');

SELECT og_cypher('hr', 'CREATE (:Person   {name: "ann", age: "31"})');
SELECT og_cypher('hr', 'CREATE (:Person   {name: "bob", age: "42"})');
SELECT og_cypher('hr', 'CREATE (:Person   {name: "cy",  age: "abc"})');
SELECT og_cypher('hr', 'CREATE (:Person   {name: "dee"})');
SELECT og_cypher('hr', 'CREATE (:Employee {name: "eve", age: "9"})');
\o

\echo '--- 문자열 비교라서 "9" > "40" 이 참이 된다 ---'
SELECT og_cypher('hr', 'MATCH (p:Person) WHERE p.age > "40" RETURN p.name AS name, p.age AS age ORDER BY name');

\echo
\echo '=== 1. age: string → int (alter_property_type) ==='
\echo '--- 1-1. plan: 아무것도 바꾸지 않고 영향만 분석한다 ---'
SELECT og_schema_plan('hr', '{"kind":"alter_property_type","type":"Person","property":"age","to":"int"}') AS c1 \gset
SELECT phase, jsonb_pretty(report) AS report FROM og_schema_changes('hr') WHERE change_id = :c1;
SELECT og_cypher('hr', 'MATCH (p) WHERE id(p) = ' ||
       (SELECT report->'uncastable_samples'->>0 FROM og_catalog.schema_change WHERE change_id = :c1) ||
       ' RETURN p.name AS name, p.age AS age');

\echo '--- 1-2. expand: 그림자 컬럼이 생기고, 새 쓰기는 즉시 새 타입 규칙을 따른다 ---'
SELECT jsonb_pretty(og_schema_expand(:c1)) AS report;
SELECT pg_temp.try($q$ SELECT og_cypher('hr', 'CREATE (:Person {name: "fay", age: "old"})') $q$) AS "age: old";
SELECT og_cypher('hr', 'CREATE (:Person {name: "gus", age: "60"})');
\echo '    (읽기는 아직 옛 컬럼을 본다 — 응용은 변화를 못 느낀다)'
SELECT og_cypher('hr', 'MATCH (p:Person {name: "gus"}) RETURN p.age AS age');

\echo '--- 1-3. migrate: 배치 크기 2 로 끝날 때까지 반복 호출 ---'
SELECT og_schema_migrate(:c1, 2);
SELECT og_schema_migrate(:c1, 2);
SELECT og_schema_migrate(:c1, 2);
SELECT og_schema_migrate(:c1, 2);

\echo '--- 1-4. validate: 변환하지 못한 행이 id 로 보고된다 ---'
SELECT og_schema_validate(:c1);
SELECT pg_temp.try(format('SELECT og_schema_contract(%s)', :c1)) AS "contract too early";

\echo '    데이터를 평소처럼 고친다 (dual write 가 그림자 컬럼도 채운다)'
SELECT og_cypher('hr', 'MATCH (p:Person {name: "cy"}) SET p.age = "38"');
SELECT og_schema_validate(:c1);

\echo '--- 1-5. contract: 옛 컬럼을 버리고 그림자 컬럼이 이름을 이어받는다 ---'
SELECT jsonb_pretty(og_schema_contract(:c1) - 'lock' - 'cursor') AS report;
\echo '    이제 숫자 비교다'
SELECT og_cypher('hr', 'MATCH (p:Person) WHERE p.age > 40 RETURN p.name AS name, p.age AS age ORDER BY name');

\echo
\echo '=== 2. name → fullName (rename_property) — 옛 이름을 쓰는 앱이 살아 있는 동안 ==='
SELECT og_schema_plan('hr', '{"kind":"rename_property","type":"Person","property":"name","new_name":"fullName"}') AS c2 \gset
SELECT og_schema_expand(:c2) ->> 'lock' AS lock;
\echo '--- 두 이름이 같은 컬럼을 가리킨다: 새 앱은 fullName 으로 쓰고, 옛 앱은 name 으로 읽는다 ---'
SELECT og_cypher('hr', 'CREATE (:Person {fullName: "hal", age: 20})');
SELECT og_cypher('hr', 'MATCH (p:Person {name: "hal"}) RETURN p.name AS name, p.fullName AS fullName');
SELECT og_cypher('hr', 'MATCH (e:Employee) RETURN e.fullName AS fullName');
\echo '--- 옛 앱이 모두 배포된 뒤 contract ---'
SELECT og_schema_validate(:c2);
SELECT og_schema_contract(:c2);
SELECT og_cypher('hr', 'MATCH (p:Person {fullName: "ann"}) RETURN p.fullName AS fullName, p.name AS name');

\echo
\echo '=== 3. age 를 필수로 (set_required, 기본값 0) ==='
SELECT og_schema_plan('hr', '{"kind":"set_required","type":"Person","property":"age","default":0}') AS c3 \gset
SELECT report->'missing' AS missing, report->'missing_samples' AS samples, report->>'lock' AS lock
  FROM og_schema_changes('hr') WHERE change_id = :c3;
SELECT og_schema_expand(:c3) IS NOT NULL AS expanded;
\echo '--- expand 직후부터 age 없는 새 쓰기는 거부된다 (기존 행은 아직 그대로) ---'
SELECT pg_temp.try($q$ SELECT og_cypher('hr', 'CREATE (:Person {fullName: "ivy"})') $q$) AS "no age";
SELECT og_schema_migrate(:c3);
SELECT og_schema_validate(:c3);
SELECT og_schema_contract(:c3) IS NOT NULL AS contracted;
SELECT og_cypher('hr', 'MATCH (p:Person {fullName: "dee"}) RETURN p.fullName AS fullName, p.age AS age');

\echo
\echo '=== 4. 한 번에: og_schema_apply ==='
\echo '--- 검증에 실패하면 아무것도 바뀌지 않는다: 기본값 없는 set_required ---'
\o /dev/null
SELECT og_add_property('hr', 'Person', 'email', 'string');
SELECT og_cypher('hr', 'MATCH (p:Person {fullName: "ann"}) SET p.email = "ann@example.com"');
\o
SELECT pg_temp.try($q$ SELECT og_schema_apply('hr', '{"kind":"set_required","type":"Person","property":"email"}') $q$) AS apply;
SELECT change_id, kind, phase FROM og_schema_changes('hr') ORDER BY change_id;
SELECT required FROM og_catalog.property WHERE type_id = og_type_id('hr', 'Person') AND name = 'email';
\echo '--- 안전한 변경은 한 번에: Employee → Staff (rename_type) ---'
SELECT og_schema_apply('hr', '{"kind":"rename_type","type":"Employee","new_name":"Staff"}');
SELECT og_cypher('hr', 'MATCH (s:Staff) RETURN s.fullName AS fullName');

\echo
\echo '=== 5. 추상 상위 타입 Party 도입 (add_parent) — 행은 한 줄도 다시 쓰지 않는다 ==='
\o /dev/null
SELECT og_create_type('hr', 'Party', 'entity', ARRAY[]::text[], true);   -- abstract
SELECT og_add_property('hr', 'Party', 'since', 'date');
\o
SELECT storage_table AS person_t FROM og_catalog.type WHERE name = 'Person' \gset
SELECT string_agg(xmin::text, ',' ORDER BY id) AS xmin_before FROM :person_t \gset
SELECT og_schema_plan('hr', '{"kind":"add_parent","type":"Person","parent":"Party"}') AS c5 \gset
SELECT report->'gains_properties' AS gains, report->'rows_touched' AS rows_touched, report->>'lock' AS lock
  FROM og_schema_changes('hr') WHERE change_id = :c5;
SELECT og_schema_expand(:c5) IS NOT NULL AS expanded;
SELECT og_schema_validate(:c5);
SELECT og_schema_contract(:c5) IS NOT NULL AS contracted;
SELECT (SELECT string_agg(xmin::text, ',' ORDER BY id) FROM :person_t) = :'xmin_before' AS rows_untouched;
SELECT count(*) AS parties FROM og_cypher('hr', 'MATCH (x:Party) RETURN x');
SELECT og_cypher('hr', 'MATCH (p:Person {fullName: "ann"}) SET p.since = "2020-03-01"');
SELECT og_cypher('hr', 'MATCH (x:Party) WHERE x.since IS NOT NULL RETURN x.fullName AS fullName, x.since AS since');

\echo
\echo '=== 6. 되돌리기 (abort) ==='
SELECT og_schema_plan('hr', '{"kind":"alter_property_type","type":"Person","property":"age","to":"string"}') AS c6 \gset
SELECT og_schema_expand(:c6) ->> 'shadow' AS shadow_column;
SELECT og_schema_abort(:c6);
SELECT count(*) AS shadow_columns_left FROM pg_attribute
 WHERE attrelid = :'person_t'::regclass AND attname LIKE 'p_age\_\_s%' AND NOT attisdropped;

\echo
\echo '=== 7. 변경 이력과 스키마 스냅샷 ==='
SELECT change_id, kind, phase, spec->>'property' AS property,
       from_version AS from_v, to_version AS to_v
  FROM og_catalog.schema_change
 WHERE graph_id = (SELECT graph_id FROM og_catalog.graph WHERE name = 'hr')
 ORDER BY change_id;
SELECT from_version AS v_before_age FROM og_catalog.schema_change WHERE change_id = :c1 \gset
\echo '--- age 를 바꾸기 전 스키마의 Person ---'
SELECT jsonb_pretty(t -> 'properties') AS person_properties
  FROM jsonb_array_elements(og_schema_snapshot('hr', :v_before_age) -> 'types') t
 WHERE t->>'name' = 'Person';
\echo '--- 지금의 Person ---'
SELECT jsonb_pretty(t -> 'properties') AS person_properties, t -> 'parents' AS parents
  FROM jsonb_array_elements(og_schema_snapshot('hr') -> 'types') t
 WHERE t->>'name' = 'Person';
