# 시험 — 두 판(v1 · age)에 나눠 돌리기

v1(지금 저장 구조, 확장 `ontological`)과 v2(Apache AGE 모양 저장, 확장 `ontological_age`)를 한 줄기에서 함께 유지한다
(`reports/ontological-v2/DESIGN.md` 5절). 시험도 그에 맞춰 셋으로 나눈다.

| 자리 | 무엇 | 어느 판에서 도나 |
|---|---|---|
| `tests/common/` | 저장 구조를 직접 보지 않는 시험 — Cypher·TypeQL·Bolt·패브릭 문장의 동작과 SQL 함수 API | 두 판 모두 |
| `tests/v1/` | v1 의 물리 구조를 직접 보는 시험(아래 기준) | v1 |
| `tests/age/` | v2 의 물리 구조를 직접 보는 시험. 지금은 비어 있다(P2 부터) | age |

`tests/run.sh` 와 `scripts/test-all.sh` 가 `common/` 과 판의 자리를 함께 돌린다. 시험 내용은 옮기기 전과 한 글자도 다르지 않다.
바뀐 것은 자리, 저장소 뿌리를 찾는 경로, 그리고 고정돼 있던 포트·덤프 경로를 환경 변수로 바꿀 수 있게 한 것뿐이다
(`OG_BOLT_TEST_LISTEN`·`OG_BOLT_BIN` — bolt-context, `OG_BOLT_URI`·`OG_BOLT_USER` — neo4j-compat-fixes,
`OG_DUMP_FILE`·`OG_EDITION` — `tests/run.sh`).

## 돌리기

```bash
# 한 번에 전부: 임시 클러스터를 만들고, 엔진을 설치하고, 모든 스위트를 돌린 뒤 요약을 낸다
scripts/test-all.sh --edition v1 --pg 16 --pg-config <PostgreSQL>/bin/pg_config --port 28816

# Studio 의 runtime/pg 처럼 쓰면 안 되는 PostgreSQL 은 작업 폴더로 복사해서 쓴다
scripts/test-all.sh --edition v1 --pg 16 --copy-pg \
    --pg-config ~/uengine-platform/ontology-studio-desktop/runtime/pg/bin/pg_config \
    --work /path/to/scratch --python /path/to/venv/bin/python

# 이미 떠 있는 서버에 SQL 회귀만
PGHOST=127.0.0.1 PGPORT=28816 PGUSER=og OG_EDITION=v1 bash tests/run.sh
```

`scripts/test-all.sh --help` 가 선택 사항을 모두 보여 준다. CI(`.github/workflows/tests.yml`)는 이 스크립트를
{v1, age} × {PG16, PG17} 으로 부른다. age 칸은 엔진이 생길 때까지 「건너뜀」으로 끝난다.

## 가르는 기준

다음 가운데 하나라도 있으면 그 파일(스위트)은 `v1/` 이다. 없으면 `common/` 이다.
시험 내용을 바꾸지 않기로 했으므로 **파일 단위로** 가른다. 한 파일 안에서 물리 확인 몇 곳만 걸리면 그 위치를 아래에 적어 두었다.
그 몇 곳을 판별 시험으로 떼어 내면(P2 이후) 나머지는 `common/` 으로 옮길 수 있다.

| 기호 | 걸리는 것 | v2 쪽 사정 (설계서 7.4·5.4) |
|---|---|---|
| **저장** | v1 저장 테이블·뷰(`og_data.og_node`·`og_edge`·`og_adj`·`n_<tid>`·`h_<tid>`·`vp_*`)나 그 열(`p_*`·`__valid`·`__actor`·`__ext`)을 직접 읽거나 쓰기. `pg_attribute`·`information_schema`·`pg_class`·`pg_indexes`·`xmin` 으로 그 구조를 보기 | v2 에 없다. 같은 뜻의 시험을 `age/` 에 새로 쓴다 |
| **감사** | `og_data.og_audit` 를 직접 읽거나 쓰기 | v2 는 자기 스키마에 같은 이름·같은 열의 호환 뷰를 둔다. 스키마 이름만 바꾸면 될 가능성이 크다 |
| **카탈로그** | `og_catalog.{type, graph, property, type_parent}` 를 직접 읽기 | 감사와 같다(호환 뷰 대상) |
| **카탈로그+** | 그 밖의 v1 카탈로그 테이블(`og_catalog.datasource`·`schema_change`·`schema_version_seq`·`role`·`typeql_function`·`embedding`) | v2 에 같은 이름이 생길지 정해지지 않았다 |
| **가상 뷰** | `og_views.*` 를 직접 보기 | 가상 라벨 뷰의 모양은 판마다 다르다(설계서 5.2) |
| **생성 SQL** | `og_cypher_sql` 이 낸 SQL 에 v1 이름(`p_*` 열 등)이 있는지 확인 | 판마다 다르다 |

`og_cypher`·`og_typeql`·`og_fabric`·`og_create_type` 같은 SQL 함수를 부르는 것은 걸리지 않는다. 두 판이 같은 함수 이름·같은 응답 꼴을
낸다는 것이 설계의 전제다(5.2절). 다만 `og_type_id`·`og_nodes`·`og_expand` 처럼 v1 의 타입 id·인접 구조를 드러내는 함수는 v2 에
없을 수 있어, `common/` 에 그런 함수를 쓰는 파일이 있으면 아래 「알아 둘 것」에 적었다.

## 분류

위치는 `파일:행 번호`(옮기기 전과 같다).

### SQL 회귀 (`tests/*/sql`, 30개 + 백업 왕복)

| 파일 | 자리 | 걸리는 것 | 위치와 내용 |
|---|---|---|---|
| `01_catalog_storage.sql` | v1 | 저장·카탈로그 | `:37` `information_schema.columns` 로 `og_data.n_<tid>` 의 열 보기 · `:42-44` `og_data.og_node`+`og_data.n_5`+`og_catalog.type` 로 `__ext` 보기 · `:55` `og_data.og_adj` |
| `02_cypher.sql` | common | — | |
| `03_vector_agent_rdf.sql` | common | — | |
| `04_neo4j_compat.sql` | common | — | |
| `05_reachability.sql` | v1 | 저장·카탈로그 | `:29-31`, `:64-66` 시작 노드 id 를 `og_data.og_node` JOIN `og_catalog.type` 로 얻음 |
| `06_correctness_regressions.sql` | common | — | |
| `07_write_security.sql` | v1 | 저장·감사·카탈로그 | `:84` `og_catalog.property` · `:108-109` `og_catalog.type.storage_table`·`type_id` · `:115` `og_data.og_node` 에 직접 INSERT(거부 확인) · `:171`·`:176`·`:223`·`:242`·`:253` `og_data.og_audit` 읽기 · `:182-183` `og_audit` DELETE/UPDATE 거부 |
| `08_schema_evolution.sql` | v1 | 저장·카탈로그·카탈로그+ | `:32-33`·`:128`·`:209`·`:240` `pg_attribute` 로 저장 테이블의 열 · `:139` `pg_indexes` · `:49-50` `storage_table` · `:59`·`:153`·`:198` `og_catalog.schema_change` · `:143`·`:189` `og_catalog.property`/`type` · `:280` `og_catalog.role` · `:292-294` `og_data.n_1` 의 `xmin`(다시 쓰기 없음 증명) |
| `09_bitemporal.sql` | v1 | 저장·카탈로그 | `:169` `storage_table` · `:173` `og_data.n_1` 직접 UPDATE · `:186-187` `og_data.h_1` 쓰기 거부 · `:191` `h_1` 읽기 · `:205`·`:208` `h_<tid>` 의 `pg_attribute`·`to_regclass` |
| `10_evolution_temporal_coverage.sql` | v1 | 저장·카탈로그+ | `:44`·`:46` `og_catalog.schema_version_seq` · `:72` `og_catalog.role` · `:82-85` `og_data.h_<tid>` 의 `pg_attribute` · `:96` `og_data.n_1.__valid` · `:143-145` `og_data.og_edge`·`og_adj` 에 직접 INSERT(거부 확인) |
| `11_release_timeline.sql` | common | — | |
| `12_view_rollback.sql` | common | — | `:4` 은 주석에 `og_data.v_N` 이 나올 뿐 |
| `13_virtual_fabric.sql` | common | — | |
| `14_fabric_statements.sql` | v1 | 감사·카탈로그+·가상 뷰 | `:87`·`:91` `og_data.og_audit` · `:88` `og_catalog.datasource` · `:107` `to_regclass('og_views.fab__spend')` |
| `15_fabric_credentials.sql` | v1 | 감사·카탈로그+ | `:62`·`:90`·`:104` `og_catalog.datasource.options` · `:108` `og_data.og_audit` |
| `16_fabric_profile.sql` | common | — | |
| `17_statement_timeout.sql` | common | — | `pg_stat_activity` 는 저장 구조와 무관 |
| `18_cypher_temporal.sql` | common | — | |
| `19_virtual_umbrella_labels.sql` | common | — | |
| `20_cypher_to_integer.sql` | common | — | |
| `21_cypher_virtual_gaps.sql` | v1 | 생성 SQL | `:218` 생성 SQL 에 `p_active = true` 가 있어야 함. 이 한 곳만 걸린다 |
| `22_wide_unions.sql` | v1 | 저장 | `:344-345` `pg_class` 에서 `og_data.vp_*` 묶음 뷰가 있는지. 이 한 곳만 걸린다 |
| `23_fabric_query.sql` | v1 | 감사·카탈로그+ | `:143-144` `QUERY FABRIC` 이 `og_catalog.datasource`·`og_data.og_audit` 읽기를 거절하는지(이름이 v1 의 것) · `:252`·`:261` `og_catalog.datasource` 권한 GRANT/REVOKE · `:283` `og_data.og_audit` 읽기 |
| `24_cypher_neo4j_semantics.sql` | common | — | |
| `25_cypher_neo4j5_syntax.sql` | common | — | |
| `26_fabric_source_catalog.sql` | v1 | 카탈로그+ | `:67` `og_catalog.datasource` 직접 UPDATE. 이 한 곳만 걸린다 |
| `27_cypher_graph_isolation.sql` | common | — | |
| `28_cypher_call_subqueries.sql` | common | — | |
| `29_fabric_inline_password_setting.sql` | v1 | 카탈로그+ | `:61` `og_catalog.datasource` 에 남은 것이 없는지. 이 한 곳만 걸린다 |
| `30_cypher_untyped_virtual_relationships.sql` | common | — | |
| 백업 왕복(`tests/run.sh` 끝) | v1 | 저장 | `og_node_view`·`og_edge_view` 로 세고 `og_check_integrity()` 로 확인. v2 는 자기 확장 테이블 등록(`pg_extension_config_dump`)을 확인하는 왕복을 따로 써야 한다 |

common 17 · v1 13 (+ 백업 왕복). 이전 자리: `engine/tests/sql/`.

### Python·그 밖의 스위트

| 스위트 | 자리 | 걸리는 것 | 위치와 내용 |
|---|---|---|---|
| `bolt-concurrency` | common | — | `:19` 은 문서 문자열에 `og_data.v_N` 이 나올 뿐 |
| `bolt-cypher-gaps` | common | — | |
| `bolt-multibyte` | common | — | |
| `bolt-params` | common | — | |
| `bolt-subqueries` | common | — | |
| `bolt-timeout` | common | — | |
| `bolt-untyped-virtual` | common | — | neo4j 드라이버 5 이상 |
| `fabric-mysql-catalog` | common | — | MySQL(또는 MariaDB) 원천과 `mysql_fdw` 가 있어야 한다. `OG_TEST_MYSQL_HOST` 가 없으면 이유를 대고 0 으로 끝난다 |
| `neo4j-movies` | common | — | 게이트웨이를 스스로 띄우지 않는다. `scripts/test-all.sh` 가 띄워 준다 |
| `neo4j-compat-fixes` | common | — | 이전 자리 `engine/tests/neo4j_compat_fixes.py`. 게이트웨이와 `fix_probe` 그래프가 있어야 한다 — `scripts/test-all.sh` 가 만든다 |
| `bolt-context` | v1 | 저장·감사 | `run.py:84`·`:94` `og_data.n_1.__valid`·`__actor` · `:86`·`:99` `og_data.og_audit` |
| `bolt-fabric-query` | v1 | 감사·카탈로그+ | `run.py:239` 거절 목록에 `og_catalog.datasource` · `:269` `og_data.og_audit` 읽기 |
| `typeql` | v1 | 저장·카탈로그·카탈로그+ | `run.py:120`·`:221` `og_catalog.type` · `:126` `og_catalog.role` · `:130` `og_catalog.typeql_function` · `:152-153` `og_catalog.type`+`og_data.og_node` · `:222` `og_catalog.graph` |

### 튜토리얼 (`examples/tutorials/*/tutorial.src.md`, 자리는 그대로)

튜토리얼은 문서의 원본이라 옮기지 않는다. `tests/tutorials.sh` 가 도는 여섯 편 모두 실행 블록 안에서 v1 테이블을 읽으므로
v1 판에서만 돌린다.

| 튜토리얼 | 판 | 실행 블록 안의 위치 |
|---|---|---|
| `agent-native` | v1 | `:290`·`:293`·`:338` `og_data.og_audit` |
| `schema-evolution` | v1 | `:98`·`:258`·`:265`·`:292` `og_data.n_1`/`n_2` · `:291` `pg_attribute` · `:304`·`:323` `og_catalog.schema_change` · `:305` `og_catalog.graph` |
| `security` | v1 | `:73`·`:147`·`:202-204`·`:234` `og_data.n_*` · `:146` `og_data.og_node` · `:196` `information_schema.column_privileges` · `:243`·`:257-258`·`:309` `og_data.og_audit` |
| `temporal` | v1 | `:254` `og_data.h_<tid>`+`og_catalog.type` · `:310` `og_data.n_1` · `:334` `og_data.og_audit` |
| `three-languages` | v1 | `:306` `og_data.og_audit` |
| `vector-search` | v1 | `:76` `og_catalog.embedding` · `:77` `og_catalog.type` |
| `virtual_fabric` | common | 걸리는 곳 없음. Studio 와 원천 DB 두 개가 있어야 해서 `FABRIC=1` 일 때만 돈다(`scripts/test-all.sh` 는 돌리지 않는다) |

## 설계서와 다른 곳

설계서 5.5 는 bolt-context, TypeDB 서점 예제(`typeql`), 튜토리얼, Python 스위트 아홉을 모두 `common/` 으로 적었다.
그 가운데 `bolt-context`·`bolt-fabric-query`·`typeql`·튜토리얼 여섯 편은 위 기준에 걸려 `v1/`(튜토리얼은 v1 판에서만 돌림)로 두었다.
시험 내용을 바꾸지 않는다는 조건에서 `common/` 에 두면 age 판에서 그 물리 확인 때문에 반드시 실패하기 때문이다.
걸리는 곳이 적어(대부분 감사·카탈로그 몇 곳), 그 확인만 판별 시험으로 떼어 내면 `common/` 으로 돌아갈 수 있다.

## 알아 둘 것

- `common/` 가운데 v1 의 타입 id·인접 구조를 드러내는 SQL 함수를 부르는 파일은 없다(`og_type_id`·`og_nodes`·`og_expand`·`og_make_id`·
  `og_node_json` 은 모두 `v1/` 파일에만 나온다). `common/` 이 쓰는 SQL 함수는 `og_cypher`·`og_cypher_sql`·`og_cypher_columns`·
  `og_cypher_stats`·`og_fabric`·`og_fabric_explain`·`og_create_graph`·`og_drop_graph`·`og_create_type`·`og_add_property`·`og_add_role`·
  `og_add_embedding`·`og_vector_search`·`og_vector_search_exact`·`og_schema`·`og_load_rdf`·`og_dump_rdf`·`og_explain_error`·`og_estimate`·
  `og_create_datasource`·`og_map_virtual`·`og_create_virtual_rel`·`og_statement_timeout`·`og_enable_temporal`·`og_release_*`·`og_schema_*`·
  `og_timeline`·`og_releases`, Python 스위트에서 더해 `og_create_index` 이다. v2 가 이 함수들을 같은 이름·같은 응답 꼴로 내야 `common/` 이 그대로 돈다.
- `engine/tests/pg_regress/` 는 pgrx 의 `cargo pgrx regress` 자리라 옮기지 않았다(지금 배선돼 있지 않다 — `docs/08_operations/04_testing_and_ci.md` §3).
- 벤치 정확성 관문(`bench/harness.py`, ADR-023)은 이번 실행기에 넣지 않았다.
