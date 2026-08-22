# 02_api — SQL 함수 계약 색인

> **이 문서가 답하는 질문**
> - 이 데이터베이스의 "API"는 정확히 무엇인가?
> - 공개 SQL 함수는 총 몇 개이고, 각각 어디에 정의되어 있는가?
> - 어떤 함수가 `STABLE`이고 어떤 함수가 `VOLATILE`인가? 병렬 실행이 가능한가?
> - 함수 이름만 알 때, 어느 문서를 봐야 하는가?

---

## 1. 사실 — 이 프로젝트의 1차 계약면

Ontological의 API는 REST가 아니라 **PostgreSQL 함수 시그니처**다.
확장을 설치하면(`CREATE EXTENSION ontological CASCADE`) 세 종류의 객체가 `public`
스키마에 생성된다.

| 계층 | 정의 위치 | 개수 | 성격 |
|---|---|---|---|
| Rust `#[pg_extern]` 함수 | `engine/src/**/*.rs` | 78 | pgrx가 `CREATE FUNCTION ... LANGUAGE c`로 생성 |
| SQL 함수 | [engine/sql/access.sql](../../engine/sql/access.sql) | 13 | 전부 `LANGUAGE sql` 또는 `plpgsql` |
| 뷰 | [engine/sql/access.sql](../../engine/sql/access.sql) | 7 | 인트로스펙션·상호운용 |

부가 표면(1차 계약면 위에 얹힌 얇은 층)은 두 개뿐이다.

| 표면 | 정의 위치 | 계약 문서 |
|---|---|---|
| Bolt 4.4 게이트웨이 | [bolt/src/session.rs](../../bolt/src/session.rs) | [09_neo4j_compat.md](09_neo4j_compat.md) |
| Studio HTTP API | [portal/server/index.js](../../portal/server/index.js) | [10_studio_http_api.md](10_studio_http_api.md) |

**결정(Decision)**: 모든 기능은 평문 SQL에서 도달 가능해야 한다.
Studio와 Bolt는 이 표면 위의 편의 계층이지 특권 경로가 아니다
([docs/api.md:3](../../docs/api.md), [bolt/src/main.rs:3](../../bolt/src/main.rs#L3)).

---

## 2. 휘발성 / 병렬 안전성 표기 규칙

pgrx `#[pg_extern(...)]` 속성이 그대로 `CREATE FUNCTION` 절이 된다.

| pgrx 속성 | 생성되는 SQL 절 |
|---|---|
| `immutable` | `IMMUTABLE` |
| `stable` | `STABLE` |
| `volatile` | `VOLATILE` |
| `parallel_safe` | `PARALLEL SAFE` |
| `parallel_restricted` | `PARALLEL RESTRICTED` |
| `parallel_unsafe` | `PARALLEL UNSAFE` |
| `strict` | `STRICT` |

**필수(Required) 해석 규칙**: 속성이 **없으면** PostgreSQL `CREATE FUNCTION`의
기본값이 적용된다 — 즉 `VOLATILE`, `PARALLEL UNSAFE`. 아래 표에서
"기본값"이라고 적힌 칸은 이 의미다.

**금지(Forbidden)**: 아래 표에 없는 휘발성 값을 추정해서 쓰지 말 것.
`STRICT` 여부는 소스에 `strict`가 **명시된 함수만** 확인되었다. 그 외 함수의
`STRICT` 여부는 pgrx의 자동 추론에 달려 있어 소스만으로는 **미확인**이다.

---

## 3. Rust 타입 → SQL 타입 대응 (코드에서 확인)

| Rust | SQL |
|---|---|
| `&str`, `&'static str` | `text` |
| `i32` / `Option<i32>` | `int4` |
| `i64` | `int8` |
| `i8` | `"char"` — [engine/sql/access.sql:197](../../engine/sql/access.sql#L197)의 `ALTER FUNCTION og_reach(int8, int4[], "char", int4, int4)`로 확정 |
| `f32` / `f64` | `float4` / `float8` |
| `bool` | `bool` |
| `JsonB` | `jsonb` |
| `Option<Vec<i32>>`, `Vec<i32>` | `int4[]` |
| `Vec<Option<String>>`, `Vec<String>` | `text[]` |
| `Vec<f64>` | `float8[]` |
| `pgrx::datum::TimestampWithTimeZone` | `timestamptz` |
| `TableIterator<'static, (name!(a, T), …)>` | `TABLE(a T, …)` |
| `SetOfIterator<'static, JsonB>` | `SETOF jsonb` |
| 반환 없음 `()` | `void` |

---

## 4. 전체 함수 색인 (78 × Rust + 13 × SQL + 7 뷰)

정렬은 카테고리 → 이름. "휘발성"·"병렬" 칸은 §2 규칙으로 읽는다.

### 4.1 그래프 · 타입 DDL → [01_graph_ddl.md](01_graph_ddl.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_create_graph` | `(name text) RETURNS int4` | 기본값 | 기본값 | [engine/src/catalog/types.rs:300](../../engine/src/catalog/types.rs#L300) |
| `og_drop_graph` | `(name text) RETURNS void` | 기본값 | 기본값 | [engine/src/catalog/types.rs:321](../../engine/src/catalog/types.rs#L321) |
| `og_create_type` | `(graph text, name text, kind text, parents text[] DEFAULT '{}', is_abstract bool DEFAULT false) RETURNS int4` | 기본값 | 기본값 | [engine/src/catalog/types.rs:348](../../engine/src/catalog/types.rs#L348) |
| `og_drop_type` | `(graph text, name text, cascade bool DEFAULT false) RETURNS void` | 기본값 | 기본값 | [engine/src/catalog/types.rs:685](../../engine/src/catalog/types.rs#L685) |
| `og_add_property` | `(graph text, type_name text, prop text, data_type text, required bool DEFAULT false, is_key bool DEFAULT false) RETURNS void` | 기본값 | 기본값 | [engine/src/catalog/types.rs:510](../../engine/src/catalog/types.rs#L510) |
| `og_create_index` | `(graph text, type_name text, prop text) RETURNS void` | 기본값 | 기본값 | [engine/src/catalog/types.rs:603](../../engine/src/catalog/types.rs#L603) |
| `og_add_role` | `(graph text, rel_type text, role text, player_type text, ordinal int4, card_min int4 DEFAULT 0, card_max int4 DEFAULT NULL) RETURNS void` | 기본값 | 기본값 | [engine/src/catalog/types.rs:625](../../engine/src/catalog/types.rs#L625) |
| `og_add_rule` | `(graph text, rel_type text, characteristic text, target_type text DEFAULT NULL) RETURNS void` | 기본값 | 기본값 | [engine/src/catalog/types.rs:657](../../engine/src/catalog/types.rs#L657) |
| `og_subtypes` | `(type_id int4) RETURNS int4[]` | `STABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/catalog/labeling.rs:192](../../engine/src/catalog/labeling.rs#L192) |
| `og_supertypes` | `(type_id int4) RETURNS int4[]` | `STABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/catalog/labeling.rs:212](../../engine/src/catalog/labeling.rs#L212) |
| `og_is_subtype` | `(sub int4, sup int4) RETURNS bool` | `STABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/catalog/labeling.rs:232](../../engine/src/catalog/labeling.rs#L232) |
| `og_relabel` | `(graph_id int4) RETURNS void` | 기본값 | `STRICT` | [engine/src/catalog/labeling.rs:247](../../engine/src/catalog/labeling.rs#L247) |

### 4.2 데이터 DML → [02_data_dml.md](02_data_dml.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_create_node` | `(graph text, type_name text, props jsonb DEFAULT '{}') RETURNS int8` | 기본값 | 기본값 | [engine/src/storage/mod.rs:246](../../engine/src/storage/mod.rs#L246) |
| `og_create_edge` | `(graph text, rel_type text, src int8, dst int8, props jsonb DEFAULT '{}') RETURNS int8` | 기본값 | 기본값 | [engine/src/storage/mod.rs:389](../../engine/src/storage/mod.rs#L389) |
| `og_set_node_props` | `(id int8, props jsonb) RETURNS void` | 기본값 | 기본값 | [engine/src/storage/mod.rs:293](../../engine/src/storage/mod.rs#L293) |
| `og_delete_node` | `(id int8) RETURNS int8` | 기본값 | 기본값 | [engine/src/storage/mod.rs:350](../../engine/src/storage/mod.rs#L350) |
| `og_delete_edge` | `(id int8) RETURNS int8` | 기본값 | 기본값 | [engine/src/storage/mod.rs:496](../../engine/src/storage/mod.rs#L496) |
| `og_add_role_player` | `(graph text, rel_type text, edge_id int8, role text, player int8) RETURNS void` | 기본값 | 기본값 | [engine/src/storage/mod.rs:531](../../engine/src/storage/mod.rs#L531) |
| `og_make_id` | `(shard int4, type_id int4, local int8) RETURNS int8` | `IMMUTABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/id.rs:88](../../engine/src/id.rs#L88) |
| `og_id_type` | `(id int8) RETURNS int4` | `IMMUTABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/id.rs:73](../../engine/src/id.rs#L73) |
| `og_id_shard` | `(id int8) RETURNS int4` | `IMMUTABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/id.rs:78](../../engine/src/id.rs#L78) |
| `og_id_local` | `(id int8) RETURNS int8` | `IMMUTABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/id.rs:83](../../engine/src/id.rs#L83) |

### 4.3 Cypher → [03_cypher.md](03_cypher.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_cypher` | `(graph text, query text, params jsonb DEFAULT '{}') RETURNS SETOF jsonb` | 기본값 | 기본값 | [engine/src/cypher/mod.rs:83](../../engine/src/cypher/mod.rs#L83) |
| `og_cypher_json` | `(graph text, query text, params jsonb DEFAULT '{}') RETURNS jsonb` | 기본값 | 기본값 | [engine/src/interop/mod.rs:35](../../engine/src/interop/mod.rs#L35) |
| `og_cypher_sql` | `(graph text, query text) RETURNS text` | `STABLE` | 기본값 | [engine/src/cypher/mod.rs:74](../../engine/src/cypher/mod.rs#L74) |
| `og_cypher_explain` | `(graph text, query text, analyze bool DEFAULT false) RETURNS jsonb` | 기본값 | 기본값 | [engine/src/cypher/mod.rs:676](../../engine/src/cypher/mod.rs#L676) |
| `og_cypher_check` | `(query text) RETURNS jsonb` | `IMMUTABLE` | `PARALLEL SAFE` | [engine/src/cypher/mod.rs:699](../../engine/src/cypher/mod.rs#L699) |
| `og_cypher_columns` | `(query text) RETURNS text[]` | `IMMUTABLE` | `PARALLEL SAFE` | [engine/src/cypher/mod.rs:717](../../engine/src/cypher/mod.rs#L717) |
| `og_cypher_stats` | `() RETURNS jsonb` | `VOLATILE` | `PARALLEL UNSAFE` | [engine/src/cypher/mod.rs:117](../../engine/src/cypher/mod.rs#L117) |

### 4.4 TypeQL → [04_typeql.md](04_typeql.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_typeql` | `(graph text, query text, _params jsonb DEFAULT '{}') RETURNS SETOF jsonb` | 기본값 | 기본값 | [engine/src/typeql/mod.rs:48](../../engine/src/typeql/mod.rs#L48) |
| `og_typeql_script` | `(graph text, script text) RETURNS int8` | 기본값 | 기본값 | [engine/src/typeql/mod.rs:99](../../engine/src/typeql/mod.rs#L99) |
| `og_typeql_sql` | `(graph text, query text) RETURNS text` | `STABLE` | 기본값 | [engine/src/typeql/mod.rs:82](../../engine/src/typeql/mod.rs#L82) |
| `og_typeql_check` | `(query text) RETURNS jsonb` | `IMMUTABLE` | `PARALLEL SAFE` | [engine/src/typeql/mod.rs:68](../../engine/src/typeql/mod.rs#L68) |
| `og_typeql_schema` | `(graph text) RETURNS text` | `STABLE` | 기본값 | [engine/src/typeql/dump.rs:10](../../engine/src/typeql/dump.rs#L10) |

### 4.5 순회 · 통계 → [05_traversal_and_stats.md](05_traversal_and_stats.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_reach` | `(src int8, etypes int4[], dir "char", minhop int4, maxhop int4) RETURNS TABLE(node int8, depth int4)` | `STABLE` | `PARALLEL RESTRICTED` | [engine/src/storage/traverse.rs:80](../../engine/src/storage/traverse.rs#L80) |
| `og_csr_build` | `(etypes int4[], dir text DEFAULT 'o') RETURNS TABLE(nodes int8, edges int8, bytes int8, build_ms float8)` | 기본값 | 기본값 | [engine/src/storage/traverse.rs:295](../../engine/src/storage/traverse.rs#L295) |
| `og_csr_reach` | `(src int8, minhop int4, maxhop int4) RETURNS TABLE(node int8, depth int4)` | `STABLE` | `PARALLEL RESTRICTED` | [engine/src/storage/traverse.rs:359](../../engine/src/storage/traverse.rs#L359) |
| `og_csr_hops` | `(src int8, dst int8, maxhop int4 DEFAULT 32) RETURNS int4` | `STABLE` | `PARALLEL RESTRICTED` | [engine/src/storage/traverse.rs:442](../../engine/src/storage/traverse.rs#L442) |
| `og_csr_stats` | `() RETURNS TABLE(built_for text, nodes int8, edges int8, bytes int8)` | 기본값 | 기본값 | [engine/src/storage/traverse.rs:322](../../engine/src/storage/traverse.rs#L322) |
| `og_csr_drop` | `() RETURNS void` | 기본값 | 기본값 | [engine/src/storage/traverse.rs:316](../../engine/src/storage/traverse.rs#L316) |
| `og_degree` | `(src int8, etype int4, dir text) RETURNS int8` | `STABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/storage/adjacency.rs:76](../../engine/src/storage/adjacency.rs#L76) |
| `og_degree_all` | `(src int8, dir text) RETURNS int8` | `STABLE` | `PARALLEL SAFE` `STRICT` | [engine/src/storage/adjacency.rs:88](../../engine/src/storage/adjacency.rs#L88) |
| `og_graph_stats` | `(graph text) RETURNS jsonb` | `STABLE` | `STRICT` | [engine/src/storage/stats.rs:11](../../engine/src/storage/stats.rs#L11) |
| `og_degree_distribution` | `(graph text) RETURNS jsonb` | `STABLE` | `STRICT` | [engine/src/storage/stats.rs:86](../../engine/src/storage/stats.rs#L86) |
| `og_reorganize` | `(graph text) RETURNS int8` | 기본값 | 기본값 | [engine/src/storage/stats.rs:121](../../engine/src/storage/stats.rs#L121) |
| `og_check_integrity` | `() RETURNS TABLE(kind text, entity_id int8, detail text)` | `STABLE` | 기본값 | [engine/src/storage/stats.rs:172](../../engine/src/storage/stats.rs#L172) |

### 4.6 벡터 · 하이브리드 검색 → [06_vector_search.md](06_vector_search.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_add_embedding` | `(graph text, type_name text, prop text, dims int4, metric text DEFAULT 'cosine', source_prop text DEFAULT NULL) RETURNS void` | 기본값 | 기본값 | [engine/src/vector/mod.rs:32](../../engine/src/vector/mod.rs#L32) |
| `og_vector_search` | `(graph text, type_name text, prop text, query text, k int4 DEFAULT 10, filter text DEFAULT NULL) RETURNS TABLE(id int8, score float8, entity jsonb)` | 기본값 | 기본값 | [engine/src/vector/mod.rs:94](../../engine/src/vector/mod.rs#L94) |
| `og_vector_search_exact` | `(graph text, type_name text, prop text, query text, k int4 DEFAULT 10) RETURNS TABLE(id int8, score float8)` | 기본값 | 기본값 | [engine/src/vector/mod.rs:411](../../engine/src/vector/mod.rs#L411) |
| `og_similar` | `(graph text, id int8, prop text, k int4 DEFAULT 10) RETURNS TABLE(id int8, score float8, entity jsonb)` | 기본값 | 기본값 | [engine/src/vector/mod.rs:158](../../engine/src/vector/mod.rs#L158) |
| `og_hybrid_search` | `(graph text, type_name text, prop text, query text, anchor int8 DEFAULT NULL, k int4 DEFAULT 10, vector_weight float8 DEFAULT 1.0, graph_weight float8 DEFAULT 1.0) RETURNS TABLE(id int8, score float8, vector_score float8, graph_score float8, entity jsonb)` | 기본값 | 기본값 | [engine/src/vector/mod.rs:222](../../engine/src/vector/mod.rs#L222) |
| `og_stale_embeddings` | `(graph text) RETURNS TABLE(entity_id int8, type_name text, prop text)` | `STABLE` | 기본값 | [engine/src/vector/mod.rs:299](../../engine/src/vector/mod.rs#L299) |
| `og_mark_embedded` | `(entity_id int8, prop text) RETURNS void` | 기본값 | 기본값 | [engine/src/vector/mod.rs:358](../../engine/src/vector/mod.rs#L358) |
| `og_embedding_stats` | `(graph text) RETURNS jsonb` | `STABLE` | `STRICT` | [engine/src/vector/mod.rs:383](../../engine/src/vector/mod.rs#L383) |

### 4.7 에이전트 인터페이스 → [07_agent_interface.md](07_agent_interface.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_schema` | `(graph text, token_budget int4 DEFAULT NULL) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/agent/mod.rs:21](../../engine/src/agent/mod.rs#L21) |
| `og_schema_for` | `(graph text, question text) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/agent/mod.rs:189](../../engine/src/agent/mod.rs#L189) |
| `og_explain_error` | `(graph text, query text) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/agent/mod.rs:261](../../engine/src/agent/mod.rs#L261) |
| `og_diagnose_empty` | `(graph text, query text) RETURNS jsonb` | 기본값 | 기본값 | [engine/src/agent/mod.rs:339](../../engine/src/agent/mod.rs#L339) |
| `og_estimate` | `(graph text, query text) RETURNS jsonb` | 기본값 | 기본값 | [engine/src/agent/mod.rs:350](../../engine/src/agent/mod.rs#L350) |
| `og_create_role` | `(name text, limits jsonb) RETURNS void` | 기본값 | 기본값 | [engine/src/agent/mod.rs:404](../../engine/src/agent/mod.rs#L404) |
| `og_apply_role` | `(name text) RETURNS jsonb` | 기본값 | 기본값 | [engine/src/agent/mod.rs:415](../../engine/src/agent/mod.rs#L415) |
| `og_enable_history` | `(graph text, type_name text) RETURNS void` | 기본값 | 기본값 | [engine/src/agent/mod.rs:448](../../engine/src/agent/mod.rs#L448) |
| `og_history` | `(id int8) RETURNS TABLE(recorded_at timestamptz, op text, payload jsonb)` | `STABLE` | 기본값 | [engine/src/agent/mod.rs:471](../../engine/src/agent/mod.rs#L471) |
| `og_as_of` | `(id int8, at timestamptz) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/agent/mod.rs:502](../../engine/src/agent/mod.rs#L502) |
| `og_set_source` | `(entity_id int8, source text, confidence float4 DEFAULT NULL, author text DEFAULT NULL) RETURNS void` | 기본값 | 기본값 | [engine/src/agent/mod.rs:529](../../engine/src/agent/mod.rs#L529) |
| `og_set_setting` | `(key text, value text) RETURNS void` | 기본값 | 기본값 | [engine/src/compat/genai.rs:55](../../engine/src/compat/genai.rs#L55) |
| `ontological_version` | `() RETURNS text` | `IMMUTABLE` | `PARALLEL SAFE` | [engine/src/lib.rs:40](../../engine/src/lib.rs#L40) |

### 4.8 상호운용 · RDF → [08_interop_and_rdf.md](08_interop_and_rdf.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_enable_rls` | `(graph text, type_name text, policy_expr text) RETURNS void` | 기본값 | 기본값 | [engine/src/interop/mod.rs:18](../../engine/src/interop/mod.rs#L18) |
| `og_map_table` | `(graph text, source_table text, type_name text, id_column text, property_map jsonb) RETURNS void` | 기본값 | 기본값 | [engine/src/interop/mod.rs:60](../../engine/src/interop/mod.rs#L60) |
| `og_materialize_mapping` | `(graph text, type_name text) RETURNS int8` | 기본값 | 기본값 | [engine/src/interop/mod.rs:120](../../engine/src/interop/mod.rs#L120) |
| `og_interop_report` | `(graph text) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/interop/mod.rs:171](../../engine/src/interop/mod.rs#L171) |
| `og_load_rdf` | `(graph text, document text, format text DEFAULT 'turtle') RETURNS jsonb` | 기본값 | 기본값 | [engine/src/adapters/mod.rs:40](../../engine/src/adapters/mod.rs#L40) |
| `og_dump_rdf` | `(graph text, format text DEFAULT 'turtle') RETURNS text` | `STABLE` | 기본값 | [engine/src/adapters/mod.rs:47](../../engine/src/adapters/mod.rs#L47) |
| `og_mapping_report` | `(graph text) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/adapters/mod.rs:53](../../engine/src/adapters/mod.rs#L53) |
| `og_add_prefix` | `(prefix text, iri text) RETURNS void` | 기본값 | 기본값 | [engine/src/adapters/mod.rs:17](../../engine/src/adapters/mod.rs#L17) |
| `og_set_iri` | `(graph text, type_name text, iri text) RETURNS void` | 기본값 | 기본값 | [engine/src/adapters/mod.rs:28](../../engine/src/adapters/mod.rs#L28) |

### 4.9 Neo4j 호환 → [09_neo4j_compat.md](09_neo4j_compat.md)

| 이름 | 시그니처 | 휘발성 | 병렬 | 정의 |
|---|---|---|---|---|
| `og_apoc_meta_schema` | `(graph text, sample int4 DEFAULT 1000) RETURNS jsonb` | `STABLE` | 기본값 | [engine/src/compat/meta.rs:184](../../engine/src/compat/meta.rs#L184) |
| `og_genai_encode` | `(resource text, provider text DEFAULT NULL, configuration jsonb DEFAULT '{}') RETURNS float8[]` | 기본값 | 기본값 | [engine/src/compat/genai.rs:95](../../engine/src/compat/genai.rs#L95) |

`CREATE INDEX` / `CREATE CONSTRAINT` 계열은 SQL 함수가 아니라 **Cypher DDL 문**으로
진입한다 — [engine/src/compat/ddl.rs:18](../../engine/src/compat/ddl.rs#L18).
`db.*` / `apoc.*` 프로시저는 `CALL ... YIELD`로만 진입한다 —
[engine/src/compat/procs.rs:80](../../engine/src/compat/procs.rs#L80).

### 4.10 SQL로 정의된 함수/뷰 ([engine/sql/access.sql](../../engine/sql/access.sql))

| 이름 | 시그니처 | 언어 / 속성 | 정의 |
|---|---|---|---|
| `og_expand` | `(src int8, etypes int4[], dir "char") RETURNS TABLE(nbr int8, eid int8)` | `sql STABLE PARALLEL SAFE ROWS 50` | [engine/sql/access.sql:14](../../engine/sql/access.sql#L14) |
| `og_expand_batch` | `(srcs int8[], etypes int4[], dir "char") RETURNS TABLE(src int8, nbr int8, eid int8)` | `sql STABLE PARALLEL SAFE ROWS 500` | [engine/sql/access.sql:29](../../engine/sql/access.sql#L29) |
| `og_subtype_ids` | `(root int4) RETURNS TABLE(type_id int4)` | `sql STABLE PARALLEL SAFE ROWS 8` | [engine/sql/access.sql:43](../../engine/sql/access.sql#L43) |
| `og_nodes` | `(root int4) RETURNS TABLE(id int8, type_id int4)` | `sql STABLE PARALLEL SAFE ROWS 1000` | [engine/sql/access.sql:53](../../engine/sql/access.sql#L53) |
| `og_edges` | `(root int4) RETURNS TABLE(id int8, type_id int4, src int8, dst int8)` | `sql STABLE PARALLEL SAFE ROWS 1000` | [engine/sql/access.sql:60](../../engine/sql/access.sql#L60) |
| `og_type_id` | `(graph text, type_name text) RETURNS int4` | `sql STABLE PARALLEL SAFE` | [engine/sql/access.sql:70](../../engine/sql/access.sql#L70) |
| `og_type_name` | `(type_id int4) RETURNS text` | `sql STABLE PARALLEL SAFE` | [engine/sql/access.sql:203](../../engine/sql/access.sql#L203) |
| `og_vlp` | `(src int8, etypes int4[], dir "char", minhop int, maxhop int) RETURNS TABLE(node int8, depth int, path int8[])` | `sql STABLE PARALLEL SAFE ROWS 100` | [engine/sql/access.sql:138](../../engine/sql/access.sql#L138) |
| `og_reach_sql` | `(src int8, etypes int4[], dir "char", minhop int, maxhop int) RETURNS TABLE(node int8, depth int)` | `sql STABLE PARALLEL SAFE ROWS 1000` | [engine/sql/access.sql:169](../../engine/sql/access.sql#L169) |
| `og_node_json` | `(id int8) RETURNS jsonb` | `plpgsql STABLE` | [engine/sql/access.sql:208](../../engine/sql/access.sql#L208) |
| `og_edge_json` | `(id int8) RETURNS jsonb` | `plpgsql STABLE` | [engine/sql/access.sql:237](../../engine/sql/access.sql#L237) |
| `og_prop` | `(id int8, prop text) RETURNS text` | `sql STABLE` | [engine/sql/access.sql:267](../../engine/sql/access.sql#L267) |
| `og_capture_history` | `() RETURNS trigger` | `plpgsql` | [engine/sql/access.sql:274](../../engine/sql/access.sql#L274) |

| 뷰 | 컬럼 | 정의 |
|---|---|---|
| `og_type_view` | `graph, type_id, name, kind, is_abstract, storage_table, iri, depth, lft, rgt, parents` | [engine/sql/access.sql:81](../../engine/sql/access.sql#L81) |
| `og_property_view` | `graph, type_name, property, data_type, column_name, required, is_key` | [engine/sql/access.sql:99](../../engine/sql/access.sql#L99) |
| `og_role_view` | `graph, relation, role, player_type, ordinal, card_min, card_max` | [engine/sql/access.sql:106](../../engine/sql/access.sql#L106) |
| `og_node_view` | `id, type_name, graph` | [engine/sql/access.sql:116](../../engine/sql/access.sql#L116) |
| `og_edge_view` | `id, type_name, graph, src, dst` | [engine/sql/access.sql:122](../../engine/sql/access.sql#L122) |
| `og_typeql_attribute` | `owner_id, owner_type, attribute_type, value, attribute_id` | [engine/sql/access.sql:307](../../engine/sql/access.sql#L307) |
| `og_typeql_role` | `relation_id, relation_type, role, player_id, player_type` | [engine/sql/access.sql:324](../../engine/sql/access.sql#L324) |

---

## 5. 실행 중인 데이터베이스에서 직접 확인하기

문서가 코드보다 뒤처졌을 가능성이 늘 있으므로, 정답은 항상 서버다.

```sql
-- Every public og_* function, exactly as installed.
SELECT p.proname || '(' || pg_get_function_arguments(p.oid) || ') -> '
                 || pg_get_function_result(p.oid) AS signature,
       p.provolatile,   -- i = immutable, s = stable, v = volatile
       p.proparallel,   -- s = safe, r = restricted, u = unsafe
       p.proisstrict
  FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
 WHERE n.nspname = 'public' AND p.proname LIKE 'og\_%'
 ORDER BY 1;
```

```sql
SELECT ontological_version();
```

---

## 6. 금지 / 필수

- **금지**: 이 색인에 없는 `og_*` 함수를 존재한다고 가정하고 호출 코드를 작성하지 말 것.
  본 색인은 `#[pg_extern]` 속성 전수 조사 결과다.
- **금지**: `og_catalog.*` / `og_data.*` 테이블에 직접 `INSERT` / `UPDATE`하지 말 것.
  쓰기 경로는 레지스트리·타입 테이블·양방향 인접 세그먼트를 **한 트랜잭션에서** 함께
  갱신한다([engine/src/storage/mod.rs:1](../../engine/src/storage/mod.rs#L1) 모듈 주석, spec 001 FR-012).
  직접 쓰면 `og_check_integrity()`가 잡아내는 불일치가 생긴다.
- **필수**: 사용자 입력은 반드시 `params jsonb` 인자로 전달할 것. Cypher/TypeQL
  질의 텍스트에 값을 문자열 연결하지 말 것(spec 003 FR-026).
- **필수**: `og_vector_search(filter)`, `og_enable_rls(policy_expr)`,
  `og_map_table(source_table)`는 **SQL 조각을 그대로 받는다**. 이 세 인자에는
  신뢰할 수 없는 입력을 절대 넣지 말 것 — [12_improvements_api.md](12_improvements_api.md) API-05 참조.

---

## 7. 관련 문서

- 오류 체계 전반 → [11_errors.md](11_errors.md)
- API 계약 개선 포인트 → [12_improvements_api.md](12_improvements_api.md)
- 원문 영문 요약 → [docs/api.md](../../docs/api.md)

<!-- affects: api, backend, data -->
<!-- requires-update: 02_api/11_errors.md, 02_api/12_improvements_api.md -->
