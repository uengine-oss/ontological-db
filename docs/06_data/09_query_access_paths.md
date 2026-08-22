# 09. 질의 접근 경로 — 어떤 질의가 어떤 인덱스를 타는가

> **이 문서가 답하는 질문**
> - 이 스키마에 존재하는 인덱스의 **완전한 목록**은 무엇인가?
> - 대표적인 질의 형태 각각이 어떤 인덱스를 타는가?
> - 어떤 접근이 순차 스캔인가, 그리고 그게 의도인가 아닌가?
> - 계획을 직접 확인하려면 무엇을 실행하는가?

**정본**: `engine/sql/bootstrap.sql`(고정 인덱스),
`engine/src/catalog/types.rs` / `vector/mod.rs` / `compat/ddl.rs` / `typeql/schema.rs`(런타임 인덱스),
`engine/src/cypher/compile.rs`(생성 SQL).

---

## 1. 인덱스 전수 목록

### 부트스트랩 인덱스 (`engine/sql/bootstrap.sql`)

| # | 관계 | 인덱스 | 정의 | 근거 | 판정 |
|---|---|---|---|---|---|
| 1 | `og_catalog.graph` | (PK) | `(graph_id)` | :23 | 사용 |
| 2 | `og_catalog.graph` | (UNIQUE) | `(name)` | :24 | 사용 — `graph_id(name)` 조회 |
| 3 | `og_catalog.type` | (PK) | `(type_id)` | :36 | 사용 — 최다 |
| 4 | `og_catalog.type` | (UNIQUE) | `(graph_id, name)` | :44 | 사용 — `try_type_id` |
| 5 | `og_catalog.type` | `type_graph_kind_idx` | `(graph_id, kind)` | :48 | 사용 — 그래프 스캔 + FK 캐스케이드 |
| 6 | `og_catalog.type` | `type_iri_idx` | `(iri) WHERE iri IS NOT NULL` | :49 | 사용 — RDF (`adapters/rdf.rs:620`). 단, `graph_id`가 선두가 아님 |
| 7 | `og_catalog.type_parent` | (PK) | `(type_id, parent_id)` | :55 | 사용 |
| 8 | `og_catalog.type_parent` | `type_parent_parent_idx` | `(parent_id)` | :57 | 사용 — FK 캐스케이드, `og_type_view` |
| 9 | `og_catalog.type_label` | (PK) | `(type_id, path_id)` | :75 | 사용 |
| 10 | `og_catalog.type_label` | `type_label_range_idx` | `(graph_id, lft, rgt)` | :79 | **핵심** — `og_subtypes` / `og_supertypes` |
| 11 | `og_catalog.type_label` | `type_label_lft_idx` | `(graph_id, lft)` | :80 | **중복** — #10의 진부분 접두사 → `DATA-04` |
| 12 | `og_catalog.property` | (PK) | `(prop_id)` | :88 | 거의 안 씀 |
| 13 | `og_catalog.property` | (UNIQUE) | `(type_id, name)` | :99 | 사용 — `plan_props`, `view_properties` |
| 14 | `og_catalog.role` | (PK) | `(role_id)` | :108 | 사용 |
| 15 | `og_catalog.role` | (UNIQUE) | `(rel_type_id, name)` | :120 | 사용 — `find_role`, `validate_roles` |
| 16 | `og_data.og_role_player` | (PK) | `(edge_id, role_id, player_id)` | :129 | 사용 |
| 17 | `og_data.og_role_player` | `og_role_player_player_idx` | `(player_id)` | :131 | 사용 — `typeql/write.rs:586` |
| 18 | `og_catalog.og_constraint` | (PK) | `(con_id)` | :137 | 거의 안 씀 |
| 19 | `og_catalog.rule` | (PK) | `(rule_id)` | :149 | 거의 안 씀 |
| 20 | `og_catalog.rule` | (UNIQUE) | `(rel_type_id, characteristic, target_type_id)` | :154 | 사용 |
| 21 | `og_catalog.typeql_function` | (PK) | `(graph_id, name)` | :170 | 사용 |
| 22 | `og_catalog.schema_version` | (PK) | `(version)` | :178 | 사용 |
| 23 | `og_data.og_adj` | (PK) | `(src, etype, dir, seq)` | :205 | **핵심** — 순회 전부 |
| 24 | `og_data.og_node` | (PK) | `(id)` | :228 | 사용 |
| 25 | `og_data.og_node` | `og_node_type_idx` | `(type_id, id)` | :231 | 사용 — index-only scan 목적 |
| 26 | `og_data.og_edge` | (PK) | `(id)` | :234 | 사용 |
| 27 | `og_data.og_edge` | `og_edge_type_idx` | `(type_id, id)` | :239 | 사용 — `og_edges`, 카운트 |
| 28 | `og_data.og_edge` | `og_edge_src_idx` | `(src)` | :240 | **거의 안 씀** — `typeql/write.rs:384-387`만 |
| 29 | `og_data.og_edge` | `og_edge_dst_idx` | `(dst)` | :241 | **사용처를 못 찾음** → `DATA-18` |
| 30 | `og_data.og_id_alloc` | (PK) | `(type_id)` | :245 | 사용 — 모든 생성 |
| 31 | `og_catalog.setting` | (PK) | `(key)` | :253 | 사용 |
| 32 | `og_catalog.embedding` | (PK) | `(emb_id)` | :267 | 거의 안 씀 |
| 33 | `og_catalog.embedding` | (UNIQUE) | `(type_id, prop)` | :273 | 사용 — `embedding_meta` |
| 34 | `og_data.og_embedding_state` | (PK) | `(entity_id, prop)` | :284 | 사용 |
| 35 | `og_catalog.compat_index` | (PK) | `(graph_id, name)` | :304 | 사용 |
| 36 | `og_data.og_history` | (PK) | `(hist_id)` | :311 | 거의 안 씀 |
| 37 | `og_data.og_history` | `og_history_entity_idx` | `(entity_id, recorded_at DESC)` | :321 | 사용 — `og_history`, `og_as_of`, 트리거의 UPDATE |
| 38 | `og_data.og_history` | `og_history_valid_idx` | `(valid_from, valid_to)` | :322 | **사용처를 못 찾음** → `DATA-14` |
| 39 | `og_data.og_source` | (PK) | `(entity_id)` | :325 | 사용 |
| 40 | `og_catalog.prefix` | (PK) | `(prefix)` | :336 | 사용 |
| 41 | `og_data.og_iri` | (PK) | `(iri)` | :342 | 사용 |
| 42 | `og_data.og_iri` | `og_iri_entity_idx` | `(entity_id)` | :344 | 사용 — `adapters/rdf.rs:797-798` |
| 43 | `og_data.og_triple_overflow` | (PK) | `(id)` | :351 | 거의 안 씀 |
| 44 | `og_data.og_triple_overflow` | `og_triple_overflow_graph_idx` | `(graph_id)` | :358 | 사용 — 매핑 리포트 |
| 45 | `og_catalog.mapping` | (PK) | `(type_id)` | :362 | 사용 |
| 46 | `og_catalog.agent_role` | (PK) | `(name)` | :373 | 사용 |
| 47 | `og_data.og_audit` | (PK) | `(audit_id)` | :381 | 거의 안 씀 |
| 48 | `og_data.og_audit` | `og_audit_at_idx` | `(at DESC)` | :390 | 사용 — Studio (`portal/server/index.js:287`) |

### 없는 인덱스 (문제가 되는 것)

| 관계 | 없는 인덱스 | 무엇이 깨지는가 |
|---|---|---|
| `og_data.og_adj` | `(etype)` | `og_drop_type`의 `DELETE ... WHERE etype = $1`이 순차 스캔 → `DATA-03` |
| `og_catalog.role` | `(player_type_id)` | FK 캐스케이드 시 전체 스캔 + `NO ACTION`이라 삭제 실패 가능 → `DATA-05` |
| `og_catalog.role` | `(parent_role_id)` | 동상 |
| `og_catalog.og_constraint` | `(type_id)` | FK 캐스케이드 시 전체 스캔, 그리고 `kind`별 조회도 전체 스캔 → `DATA-05` |
| `og_catalog.rule` | `(target_type_id)` | FK 캐스케이드 시 전체 스캔 → `DATA-05` |
| `og_data.n_*.__ext` | GIN | 미선언 프로퍼티 필터가 전체 스캔 → `DATA-08` |

### 런타임 생성 인덱스

| 이름 패턴 | 정의 | 생성 지점 |
|---|---|---|
| (익명) | `og_data.e_<tid> (src)` | `engine/src/catalog/types.rs:429` |
| (익명) | `og_data.e_<tid> (dst)` | `engine/src/catalog/types.rs:430` |
| `e_<tid>_src` / `e_<tid>_dst` | 같은 것 — TypeQL `$has` 경로 | `engine/src/typeql/schema.rs:542-543` |
| `ix_<sub>_<col>` | `og_create_index(graph, type, prop)` | `engine/src/catalog/types.rs:610` |
| `uq_<sub>_<col>` | `is_key = true` 프로퍼티 | `engine/src/catalog/types.rs:573-575` |
| (익명 UNIQUE) | 상속된 `is_key` 프로퍼티 | `engine/src/catalog/types.rs:488` |
| (UNIQUE, 인라인) | `og_data.a_<tid> (val)` | `engine/src/typeql/schema.rs:275` |
| `hnsw_<sub>_<col>` | `USING hnsw (<col> <opclass>)` | `engine/src/vector/mod.rs:58-61` |
| `ftx_<sub>_<name>` | `USING gin (to_tsvector('simple', …))` | `engine/src/compat/ddl.rs:263-267` |

**엣지 타입 테이블의 `(src)`/`(dst)` 인덱스는 중복 생성될 수 있다** —
`create_type_inner`가 익명으로 하나씩 만들고(`engine/src/catalog/types.rs:429-430`),
TypeQL `ensure_has_type`은 `e_<tid>_src` / `e_<tid>_dst`라는 이름으로 만든다
(`engine/src/typeql/schema.rs:542-543`). `$has`는 카탈로그에 직접 INSERT되어
`create_type_inner`를 거치지 않으므로 실제 중복은 없지만, 두 경로가 나뉘어 있다.

---

## 2. 질의 형태별 접근 경로

### Q1. 한 홉 확장 — `MATCH (a:P)-[:K]->(b:P)`

컴파일된 SQL (`engine/src/cypher/compile.rs:900-904`):
```sql
CROSS JOIN LATERAL (
  SELECT u.nbr, u.eid FROM og_data.og_adj adj1,
         LATERAL unnest(adj1.nbr, adj1.eid) AS u(nbr, eid)
   WHERE adj1.src = a0.id AND adj1.dir = 'o'::"char" AND adj1.etype = ANY($types)
) u1
```

| 조건 | 인덱스 | 역할 |
|---|---|---|
| `src = a0.id` | `og_adj` PK 1번 컬럼 | 경계 조건 |
| `etype = ANY(...)` | PK 2번 컬럼 | 경계 조건 (ScalarArrayOp) |
| `dir = 'o'` | PK 3번 컬럼 | 경계 조건 |

**힙 페이지 접근**: 해당 `(src, etype, dir)`의 세그먼트 수만큼. 차수 ≤ 256이면 1개.

### Q2. 라벨 스캔 — `MATCH (a:P) WHERE a.val = 42`

컴파일러가 `v_<tid>` 뷰를 만든다(`engine/src/cypher/views.rs:93-138`).
뷰는 구체 테이블들의 `UNION ALL`이므로, 각 분기에서:

| 조건 | 인덱스 |
|---|---|
| `p_val = 42` | `ix_<sub>_p_val` — **`og_create_index()`를 불렀을 때만 존재** |
| (없으면) | 각 분기 순차 스캔 |

벤치 하네스가 이 인덱스를 명시적으로 만드는 이유다(`bench/harness.py:359`).

**주의**: `og_create_index(graph, type, prop)`는 `og_subtypes(tid)`를 돌며
각 구체 테이블에 인덱스를 만든다(`engine/src/catalog/types.rs:608-613`).
**나중에 만들어진 서브타입에는 인덱스가 안 붙는다.**

### Q3. 서브타입 판정 — `og_is_subtype(sub, sup)` / `og_subtypes(root)`

```sql
SELECT DISTINCT d.type_id
  FROM og_catalog.type_label a
  JOIN og_catalog.type_label d
    ON d.graph_id = a.graph_id AND d.lft >= a.lft AND d.rgt <= a.rgt
 WHERE a.type_id = $1
```
(`engine/src/catalog/labeling.rs:197-202`)

| 관계 | 접근 |
|---|---|
| `a` | `type_label` PK `(type_id, path_id)` — 등호 프로브 |
| `d` | `type_label_range_idx (graph_id, lft, rgt)` — `graph_id` 등호 + `lft >=` 범위. `rgt <=`는 인덱스 내 필터 |

`type_label_lft_idx`는 여기서 쓰일 수 있지만 `range_idx`가 항상 최소한 같거나 낫다
(추가 컬럼 `rgt`가 인덱스 내 필터를 가능하게 하므로 힙 접근이 줄어든다).

### Q4. 타입별 인스턴스 스캔 — `og_nodes(root)`

```sql
SELECT n.id, n.type_id FROM og_data.og_node n
 WHERE n.type_id IN (SELECT og_subtype_ids(og_nodes.root))
```
(`engine/sql/access.sql:56-57`)

`og_node_type_idx (type_id, id)` — `type_id` 등호(또는 IN) + `id`가 인덱스에 있어
**index-only scan**이 가능하다. `(type_id)`만이 아니라 `(type_id, id)`인 이유다.

### Q5. id로 노드 하나 — `og_node_json(id)`

```sql
SELECT ty.type_id, ty.name, ty.storage_table
  FROM og_data.og_node n JOIN og_catalog.type ty ON ty.type_id = n.type_id
 WHERE n.id = og_node_json.id;
EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t.storage_table) ...;
SELECT jsonb_object_agg(p.name, raw -> p.column_name) FROM og_catalog.property p
 WHERE p.type_id = t.type_id AND raw ? p.column_name AND ...;
```
(`engine/sql/access.sql:208-235`)

| 단계 | 인덱스 |
|---|---|
| `og_node WHERE id = ?` | PK |
| `og_catalog.type WHERE type_id = ?` | PK |
| `{storage_table} WHERE x.id = ?` | 타입 테이블 PK |
| `og_catalog.property WHERE type_id = ?` | `UNIQUE (type_id, name)` 선두 |

**전부 인덱스를 타지만 서브질의가 네 개다.** `LANGUAGE plpgsql`이므로
**인라인되지 않고**, 행마다 한 번씩 돈다. `to_jsonb(x)`는 `vector(N)`이나 큰 `__ext`가
있으면 TOAST를 펼친다. → `PERF-08`

### Q6. 미선언 프로퍼티 필터

타입이 알려진 경우 (`engine/src/cypher/compile.rs:989`):
```sql
(v0.__ext->>'foo') = 'bar'
```
→ **인덱스 없음. 순차 스캔.** `__ext`에 GIN 인덱스를 만드는 코드가 없다.

타입 미상인 경우 (`engine/src/cypher/compile.rs:991`):
```sql
(og_node_json(n0.id)->>'foo') = 'bar'
```
→ **인덱스 없음 + 행마다 Q5 전체.** 가장 비싼 형태다.

### Q7. 벡터 검색

```sql
SELECT v.id, ... FROM og_data.v_<tid> v
 WHERE v.p_embedding IS NOT NULL AND (<filter>)
 ORDER BY v.p_embedding <=> $1::vector LIMIT k
```
(`engine/src/vector/mod.rs:126-132`)

각 `UNION ALL` 분기에서 `hnsw_<sub>_p_embedding`을 탄다 —
`ORDER BY <거리연산자> LIMIT k` 형태가 HNSW가 인식하는 유일한 형태다.

**`ORDER BY` 표현식이 인덱스의 opclass와 일치해야 한다.**
`og_catalog.embedding.metric`을 바꿔도 인덱스는 안 바뀌므로(→ [`07`](07_vector_data_model.md))
어긋나면 조용히 순차 스캔 + 정렬로 떨어진다.

### Q8. 깊은 순회 — `og_reach` / `og_vlp` / `og_reach_sql`

| 함수 | 구현 | `og_adj` 접근 |
|---|---|---|
| `og_vlp` | SQL 재귀 CTE, 경로 배열 유지 | 레벨마다 `src = w.node` 프로브 |
| `og_reach_sql` | SQL 재귀 CTE, `UNION`(중복 제거) | 동상 |
| `og_reach` | Rust, 방문 집합, 준비된 계획 1개 | 레벨마다 `src = ANY($frontier)` — **배치** |
| `og_csr_reach` | 백엔드 로컬 CSR | `og_adj` 접근 없음 (빌드 시 1회 전체 스캔) |

`og_reach`의 SQL (`engine/src/storage/traverse.rs:94-97`):
```sql
SELECT a.nbr FROM og_data.og_adj a WHERE a.src = ANY($1::int8[]) AND <dir/etype>
```
프론티어 전체를 한 문장으로 보낸다. **한 SPI 연결, 한 계획**을 walk 전체에 재사용한다
(`engine/src/storage/traverse.rs:99-106`) — 지름이 큰 그래프에서 레벨당 재계획이
같은 walk를 재귀 CTE보다 10배 느리게 만들었던 측정이 이 결정의 근거다.

수치는 `docs/deep-traversal.md` 참조.

### Q9. `og_typeql_attribute` 뷰 스캔

(`engine/sql/access.sql:307-318`)

| 조인 | 접근 |
|---|---|
| `og_catalog.type ht ON ... AND ht.name = '$has'` | **`name` 단독 조건 — UNIQUE는 `(graph_id, name)`이라 접두사가 아님 → 순차 스캔** |
| `og_data.og_edge e ON e.type_id = ht.type_id` | `og_edge_type_idx` |
| `og_data.og_node o/a ON id = e.src/dst` | PK |
| `og_node_json(e.dst) ->> 'val'` | **행마다 Q5 전체** |

이 뷰를 넓게 스캔하는 것은 비싸다 → `PERF-08`.

---

## 3. 의도된 전체 스캔

아래는 인덱스 부재가 **결함이 아니다.**

| 위치 | 무엇 | 왜 의도인가 |
|---|---|---|
| `engine/src/storage/traverse.rs:244-247` | `og_csr_build`가 `og_adj` 전체 읽기 | 그래프 전체를 컴파일하는 것이 목적 |
| `engine/src/storage/stats.rs:126-135` | `og_reorganize` 대상 선정 | 전역 조각화 분석 |
| `engine/src/storage/stats.rs:49-52` | `og_graph_stats`의 세그먼트 집계 | 전역 통계 |
| `engine/src/storage/stats.rs:92-100` | `og_degree_distribution` | 전역 히스토그램 |
| `engine/src/catalog/labeling.rs:36-56` | `load_dag`의 카탈로그 전체 읽기 | 계층 전체 재계산 |
| `engine/src/vector/mod.rs:336-341` | `og_stale_embeddings` | 배치 작업 |
| `engine/src/catalog/types.rs:561-567` | `og_add_property`의 `__ext` 백필 | 일회성 DDL |

---

## 4. 계획 확인 방법

### 컴파일된 Cypher SQL 보기
```sql
SELECT og_cypher_sql('mygraph', $$ MATCH (a:P)-[:K]->(b:P) WHERE a.val = 1 RETURN count(b) $$);
```
(`engine/src/cypher/mod.rs`의 `og_cypher_sql`)

### 실행 계획
```sql
SELECT og_cypher_explain('mygraph', $$ ... $$, true);   -- true = ANALYZE, BUFFERS
```
(`engine/src/cypher/mod.rs:677-682`)

### 비용 추정 + 조언
```sql
SELECT og_estimate('mygraph', $$ ... $$);
```
(`engine/src/agent/mod.rs:350-382`) — 추정 행 수가 100만을 넘거나
연결되지 않은 패턴이 있으면 문장으로 경고한다.

### 인덱스가 실제로 쓰이는지
```sql
SELECT relname, indexrelname, idx_scan, idx_tup_read, idx_tup_fetch
  FROM pg_stat_user_indexes
 WHERE schemaname IN ('og_data', 'og_catalog')
 ORDER BY idx_scan ASC;     -- idx_scan = 0 인 것이 안 쓰이는 인덱스
```

이것이 위 1절의 "사용처를 못 찾음" 판정을 **당신의 워크로드에서** 확인하는 방법이다.
코드를 읽어서 내린 판정이므로, 사용자 SQL이 직접 쓰고 있을 수 있다.

### `og_adj` 세그먼트 실측
```sql
SELECT etype, dir, count(*) AS segments, avg(n)::numeric(6,1) AS avg_fill,
       max(n) AS max_fill, sum(n) AS total_neighbours
  FROM og_data.og_adj GROUP BY etype, dir ORDER BY 5 DESC;
```

---

## 금지 / 필수

**금지**
- 미선언 프로퍼티(`__ext`)로 필터링하는 질의를 뜨거운 경로에 두는 것.
- 타입 미상 변수(`MATCH (n) WHERE n.x = ...`)로 프로퍼티를 읽는 것.
- `og_typeql_attribute` 뷰를 조인의 큰 쪽으로 쓰는 것.
- 인덱스를 추가하면서 이 문서를 갱신하지 않는 것.

**필수**
- 필터에 쓰이는 프로퍼티는 `og_create_index(graph, type, prop)`로 인덱스를 만들 것.
- **서브타입을 나중에 추가했으면 `og_create_index()`를 다시 부를 것.**
  기존 인덱스는 새 서브타입 테이블에 자동으로 생기지 않는다.
- 벌크 로드 후 `ANALYZE`. 통계가 없으면 깊은 순회 전환 판단이
  "깊이 ≥ 4" 규칙으로 폴백한다(`engine/src/cypher/compile.rs:51-54`).
- 새 접근 경로를 만들면 이 문서의 1절과 2절에 행을 추가할 것.

---

<!-- affects: data, backend, performance -->
<!-- requires-update: docs/06_data/10_improvements_data.md -->
