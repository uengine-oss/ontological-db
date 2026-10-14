-- Bitemporal tutorial — "what was true then" vs "what did we believe then".
--
-- Companion to docs/tutorials/temporal-tutorial.md. Run it against a fresh
-- database that has the extension installed:
--
--   createdb tut_temporal
--   psql -d tut_temporal -c 'CREATE EXTENSION ontological CASCADE'
--   psql -d tut_temporal -f examples/tutorials/temporal.sql
--
-- Valid time is set explicitly (og.valid_from). System time is the real clock,
-- so every write runs in its own transaction and records its now() in a temp
-- table `marks` — that is how the script can later ask "as we believed right
-- after write W2". Your timestamps will differ from the tutorial's; the labels
-- (W1..W4) and the answers will not.
\set ON_ERROR_STOP on
\set QUIET on
\pset footer off
SET client_min_messages = warning;

CREATE TEMP TABLE marks (label text, at timestamptz);
-- A system-time instant as the write that produced it.
CREATE FUNCTION pg_temp.w(t timestamptz) RETURNS text LANGUAGE sql AS $$
    SELECT CASE WHEN t = 'infinity' THEN '∞'
                ELSE COALESCE((SELECT label FROM marks WHERE at = t), t::text) END
$$;
CREATE FUNCTION pg_temp.try(q text) RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    EXECUTE q;
    RETURN 'ok (no error)';
EXCEPTION WHEN others THEN
    RETURN SQLSTATE || ' ' || SQLERRM;
END $$;
-- The two questions the tutorial keeps asking.
CREATE FUNCTION pg_temp.salary() RETURNS jsonb LANGUAGE sql AS $$
    SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"}) RETURN p.salary AS salary')
$$;
CREATE FUNCTION pg_temp.job() RETURNS jsonb LANGUAGE sql AS $$
    SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"})-[w:WORKS_AT]->(c:Company)
                              RETURN c.name AS company, w.title AS title')
$$;

\echo
\echo '=== 0. 스키마: Person, Company, WORKS_AT 을 시간 버전 관리 대상으로 ==='
\o /dev/null
SELECT og_create_graph('corp');
SELECT og_create_type('corp', 'Person', 'entity');
SELECT og_add_property('corp', 'Person', 'name', 'string');
SELECT og_add_property('corp', 'Person', 'salary', 'int');
SELECT og_create_type('corp', 'Company', 'entity');
SELECT og_add_property('corp', 'Company', 'name', 'string');
SELECT og_create_type('corp', 'WORKS_AT', 'relation');
SELECT og_add_property('corp', 'WORKS_AT', 'title', 'string');
SELECT og_enable_temporal('corp', 'Person');
SELECT og_enable_temporal('corp', 'Company');
SELECT og_enable_temporal('corp', 'WORKS_AT');
\o
SELECT og_temporal_types('corp');

\echo
\echo '=== W1. 2020-01-01 입사, 연봉 100 (HR 시스템이 입력) ==='
BEGIN;
SET LOCAL og.valid_from = '2020-01-01';
SET LOCAL og.actor  = 'kim@hr';
SET LOCAL og.source = 'hr-portal';
SELECT og_cypher('corp', 'CREATE (:Person {name: "ann", salary: 100})');
SELECT og_cypher('corp', 'CREATE (:Company {name: "acme"})');
SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"}), (c:Company {name: "acme"})
                          CREATE (p)-[:WORKS_AT {title: "engineer"}]->(c)');
INSERT INTO marks VALUES ('W1', now());
COMMIT;
SELECT pg_sleep(0.01) \g /dev/null

\echo
\echo '=== W2. 2023-01-01 부로 200 인상 — 오늘에야 입력(소급) ==='
BEGIN;
SET LOCAL og.valid_from = '2023-01-01';
SET LOCAL og.actor = 'kim@hr';
SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"}) SET p.salary = 200');
INSERT INTO marks VALUES ('W2', now());
COMMIT;
SELECT pg_sleep(0.01) \g /dev/null

\echo
\echo '=== W3. 정정: 인상액은 200 이 아니라 250 이었다 (valid_from = -infinity) ==='
BEGIN;
SET LOCAL og.valid_from = '-infinity';
SET LOCAL og.actor  = 'lee@payroll';
SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"}) SET p.salary = 250');
INSERT INTO marks VALUES ('W3', now());
COMMIT;
SELECT pg_sleep(0.01) \g /dev/null

\echo
\echo '=== W4. 2024-06-01 퇴사 (관계를 그 날짜로 삭제) ==='
BEGIN;
SET LOCAL og.valid_from = '2024-06-01';
SET LOCAL og.actor = 'kim@hr';
SELECT og_cypher('corp', 'MATCH (:Person {name: "ann"})-[w:WORKS_AT]->() DELETE w');
INSERT INTO marks VALUES ('W4', now());
COMMIT;

\echo
\echo '=== 1. 지금 ==='
SELECT pg_temp.salary() AS salary, pg_temp.job() AS job;

\echo
\echo '=== 2. valid time — "그때 실제로 어땠나" (지금 알고 있는 사실 기준) ==='
SET og.valid_at = '2019-06-01';
SELECT '2019-06-01' AS valid_at, pg_temp.salary() AS salary, pg_temp.job() AS job;
SET og.valid_at = '2021-06-01';
SELECT '2021-06-01' AS valid_at, pg_temp.salary() AS salary, pg_temp.job() AS job;
SET og.valid_at = '2023-06-01';
SELECT '2023-06-01' AS valid_at, pg_temp.salary() AS salary, pg_temp.job() AS job;
SET og.valid_at = '2024-07-01';
SELECT '2024-07-01' AS valid_at, pg_temp.salary() AS salary, pg_temp.job() AS job;
RESET og.valid_at;

\echo
\echo '=== 3. system time — "그때 우리는 무엇을 믿고 있었나" ==='
SET og.valid_at = '2023-06-01';
SELECT at AS w1 FROM marks WHERE label = 'W1' \gset
SELECT at AS w2 FROM marks WHERE label = 'W2' \gset
SELECT at AS w3 FROM marks WHERE label = 'W3' \gset
SELECT set_config('og.system_at', :'w1', false) IS NOT NULL AS system_at_w1;
SELECT '2023-06-01 @ W1' AS asked, pg_temp.salary() AS salary;
SELECT set_config('og.system_at', :'w2', false) IS NOT NULL AS system_at_w2;
SELECT '2023-06-01 @ W2' AS asked, pg_temp.salary() AS salary;
SELECT set_config('og.system_at', :'w3', false) IS NOT NULL AS system_at_w3;
SELECT '2023-06-01 @ W3' AS asked, pg_temp.salary() AS salary;
SET og.valid_at = '2024-07-01';
SELECT '2024-07-01 @ W3' AS asked, pg_temp.job() AS job;
RESET og.valid_at;
RESET og.system_at;

\echo
\echo '=== 4. 이력 테이블 들여다보기 ==='
SELECT storage_table AS person_t, 'og_data.h_' || type_id AS person_h
  FROM og_catalog.type WHERE name = 'Person' \gset
\echo '--- 현재 행 (저장 테이블) ---'
SELECT p_salary AS salary, lower(__valid)::date AS valid_from, upper(__valid) AS valid_to,
       pg_temp.w(__sys_from) AS sys_from
  FROM :person_t;
\echo '--- 대체된 믿음과 과거 구간 (이력 테이블) ---'
SELECT p_salary AS salary, lower(__valid)::date AS valid_from, upper(__valid)::text AS valid_to,
       pg_temp.w(__sys_from) AS sys_from, pg_temp.w(__sys_to) AS sys_to, __op AS op
  FROM :person_h ORDER BY __sys_from, __sys_to, lower(__valid);

\echo
\echo '=== 5. 누가, 어디서: og_meta 와 감사 로그 ==='
SELECT og_meta((og_cypher('corp', 'MATCH (p:Person {name: "ann"}) RETURN id(p) AS i')->>'i')::int8)
       - 'created_at' - 'updated_at' - 'sys_from' AS meta;
SELECT actor, source, left(query, 60) AS query
  FROM og_data.og_audit WHERE actor IS NOT NULL ORDER BY audit_id;

\echo
\echo '=== 6. 시점을 모르는 질의 모양은 거짓 대신 오류를 낸다 ==='
SET og.valid_at = '2021-06-01';
SELECT pg_temp.try($q$ SELECT og_cypher('corp', 'MATCH (n) RETURN count(n)') $q$) AS unlabelled;
SELECT pg_temp.try($q$ SELECT og_cypher('corp', 'MATCH (p:Person)-[:WORKS_AT*1..2]->(c:Company) RETURN c') $q$) AS var_length;
SELECT pg_temp.try($q$ SELECT og_typeql('corp', 'match $p isa Person;') $q$) AS typeql;
\echo '--- 라벨을 붙이고 한 홉으로 쓰면 된다 ---'
SELECT og_cypher('corp', 'MATCH (p:Person)-[:WORKS_AT]->(c:Company) RETURN p.name AS person, c.name AS company');
RESET og.valid_at;

\echo
\echo '=== 7. 세 번째 축: 그 행은 어떤 스키마 아래서 쓰였나 ==='
\o /dev/null
SELECT og_add_property('corp', 'Person', 'email', 'string');
\o
SELECT t -> 'properties' ? 'email' AS person_has_email_at_w1
  FROM jsonb_array_elements(og_schema_snapshot('corp', at => :'w1') -> 'types') t
 WHERE t->>'name' = 'Person';
SELECT t -> 'properties' ? 'email' AS person_has_email_now
  FROM jsonb_array_elements(og_schema_snapshot('corp') -> 'types') t
 WHERE t->>'name' = 'Person';
\echo '--- 이력 테이블도 새 컬럼을 받는다 (과거 행은 NULL) ---'
SELECT count(*) AS history_rows, count(p_email) AS with_email FROM :person_h;
