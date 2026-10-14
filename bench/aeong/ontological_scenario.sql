-- AeonG와 같은 이야기를 Ontological에서: 입사(2020) → 연봉 인상(2023) → 인상액 정정 → 퇴사(2024-06)
\set ON_ERROR_STOP on
\pset footer off
SELECT og_create_graph('hr');
SELECT og_create_type('hr','Person','entity');  SELECT og_add_property('hr','Person','name','string');
SELECT og_add_property('hr','Person','salary','int');
SELECT og_create_type('hr','Company','entity'); SELECT og_add_property('hr','Company','name','string');
SELECT og_create_type('hr','WORKS_AT','relation'); SELECT og_add_property('hr','WORKS_AT','title','string');
SELECT og_enable_temporal('hr','Person'); SELECT og_enable_temporal('hr','Company'); SELECT og_enable_temporal('hr','WORKS_AT');

\echo '== T1: 2020-01-01 입사 (salary 100) =='
SET og.valid_from = '2020-01-01';
SELECT og_cypher('hr','CREATE (:Person {name:"ann", salary:100})');
SELECT og_cypher('hr','CREATE (:Company {name:"acme"})');
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}),(c:Company) CREATE (p)-[:WORKS_AT {title:"eng"}]->(c)');
RESET og.valid_from;
SELECT pg_sleep(0.05); SELECT now() AS t1 \gset

\echo '== T2: 2023-01-01부터 연봉 200 (실제 입력은 오늘) =='
SET og.valid_from = '2023-01-01';
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) SET p.salary = 200');
RESET og.valid_from;
SELECT pg_sleep(0.05); SELECT now() AS t2 \gset

\echo '== T3: 정정 — 2023 인상액은 사실 250이었다 =='
SET og.valid_from = '2023-01-01';
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) SET p.salary = 250');
RESET og.valid_from;
SELECT pg_sleep(0.05); SELECT now() AS t3 \gset

\echo '== T4: 2024-06-01 퇴사 (관계 삭제) =='
SET og.valid_from = '2024-06-01';
SELECT og_cypher('hr','MATCH (:Person {name:"ann"})-[w:WORKS_AT]->() DELETE w');
RESET og.valid_from;

\echo '== Q1 현재 상태 =='
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) OPTIONAL MATCH (p)-[w:WORKS_AT]->(c:Company) RETURN p.salary AS salary, c.name AS company');

\echo '== Q2 유효시간 as-of 2021-06-01 (그때 실제로는?) =='
SET og.valid_at = '2021-06-01';
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"})-[w:WORKS_AT]->(c:Company) RETURN p.salary AS salary, c.name AS company, w.title AS title');
\echo '== Q3 유효시간 as-of 2023-06-01 =='
SET og.valid_at = '2023-06-01';
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"})-[w:WORKS_AT]->(c:Company) RETURN p.salary AS salary, c.name AS company');
\echo '== Q4 바이템포럴: 유효 2023-06-01, 정정 전(T2)에 DB가 알던 값 =='
SELECT set_config('og.system_at', :'t2', false);
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) RETURN p.salary AS salary');
\echo '== Q5 시스템시간 as-of T1 (유효시간 2023-06-01) — 아직 인상이 입력되기 전 =='
SELECT set_config('og.system_at', :'t1', false);
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) RETURN p.salary AS salary');
RESET og.system_at;
\echo '== Q6 유효시간 as-of 2025-01-01 — 퇴사 후 =='
SET og.valid_at = '2025-01-01';
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"})-[w:WORKS_AT]->(c:Company) RETURN c.name AS company');
RESET og.valid_at;

\echo '== Q7 이력(range): og_timeline =='
SELECT valid_from::date, valid_to::date, props->>'salary' AS salary, op,
       to_char(recorded_at,'HH24:MI:SS.MS') rec, to_char(superseded_at,'HH24:MI:SS.MS') sup
  FROM og_timeline('hr','Person','name','ann') ORDER BY recorded_at, valid_from;
\echo '== Q7b 같은 이력을 T2 시점의 믿음으로 =='
SELECT valid_from::date, valid_to::date, props->>'salary' AS salary, op
  FROM og_timeline('hr','Person','name','ann', system_at => :'t2') ORDER BY valid_from;
\echo '== Q7c 이력 테이블 원본(h_*) — 모든 버전 =='
SELECT __valid, p_salary AS salary, __op, to_char(__sys_from,'HH24:MI:SS.MS') sys_from, to_char(__sys_to,'HH24:MI:SS.MS') sys_to FROM og_data.h_1 ORDER BY __sys_from;

\echo '== Q8 as-of 상태에서 지원하지 않는 형태 (가변 길이 경로) =='
SET og.valid_at = '2021-06-01';
DO $$ BEGIN PERFORM og_cypher('hr','MATCH (p:Person)-[:WORKS_AT*1..2]->(c:Company) RETURN c.name');
EXCEPTION WHEN others THEN RAISE NOTICE '% %', SQLSTATE, SQLERRM; END $$;
RESET og.valid_at;

\echo '== 저장 구조: 현재 테이블 vs 이력 테이블 =='
SELECT c.relname, c.reltuples::int, pg_size_pretty(pg_total_relation_size(c.oid)) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname='og_data' AND c.relkind='r' ORDER BY 1;

\echo '== Q9 (AeonG P2/P4 대응) 사람 노드를 지운 뒤에도 과거는 보이는가 =='
SELECT now() AS before_delete \gset
SELECT pg_sleep(0.05);
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) DETACH DELETE p');
\echo '-- 현재'
SELECT og_cypher('hr','MATCH (p:Person) RETURN p.name AS name');
\echo '-- 유효시간 2021-06-01 (그리고 시스템시간은 지금): 삭제는 "지금부터 없음"이므로 2021 년의 ann 은 그대로'
SET og.valid_at = '2021-06-01';
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"})-[w:WORKS_AT]->(c:Company) RETURN p.salary AS salary, c.name AS company');
\echo '-- 시스템시간 = 삭제 직전, 유효시간 2023-06-01'
SET og.valid_at = '2023-06-01';
SELECT set_config('og.system_at', :'before_delete', false);
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) RETURN p.salary AS salary');
RESET og.system_at; RESET og.valid_at;
