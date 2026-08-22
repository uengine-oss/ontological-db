# 핫패스 해부 — 코드를 세어 산출한 비용

> **이 문서가 답하는 질문**
> - 읽기 한 번에 정확히 무엇이 일어나는가: 라벨 스캔 → 인접 언네스트 → 조인 → 프로젝션
> - 쓰기 한 번에 정확히 무엇이 일어나는가: id 할당 → 프로퍼티 계획 → 타입 테이블 INSERT → 양방향 인접 갱신
> - **노드 1개를 만드는 데 SPI 호출이 몇 번 일어나는가?**
> - 각 단계에서 페이지는 몇 번 접근되는가?

---

## 0. 규칙 — 이 문서의 셈법

- SPI 왕복 1회 = `Spi::connect(...)` / `Spi::run_with_args(...)` / `Spi::run(...)` 호출 1개.
  [`engine/src/spiu.rs:15-48`](../../../engine/src/spiu.rs) 의 `one` / `two` / `one_mut` 은 각각
  독립적인 `Spi::connect` 이므로 **왕복 1회씩** 센다.
- 이 코드베이스에서 준비된 계획(prepared plan)을 재사용하는 곳은 단 한 곳,
  `og_reach` 의 레벨 루프뿐이다 ([`traverse.rs:107-116`](../../../engine/src/storage/traverse.rs)).
  나머지는 모두 매 호출마다 SQL 텍스트를 넘긴다.
- "콜드" = 이 백엔드에서 해당 질의가 처음 컴파일되는 경우(`PLAN_CACHE` 미스).
  "웜" = `PLAN_CACHE` 히트 ([`cypher/mod.rs:47-67`](../../../engine/src/cypher/mod.rs)).

## 1. 사실 — 읽기 핫패스의 전체 경로

```
og_cypher(graph, query, params)                       engine/src/cypher/mod.rs:84
  ├─ stats::reset()                                   SPI 0
  ├─ parser::parse(query)                             SPI 0   ← 1차 파싱
  ├─ is_write(&ast)                                   SPI 0
  ├─ run_read()                        mod.rs:137
  │    ├─ compile_cached()             mod.rs:47
  │    │    ├─ [캐시 히트] → 종료                      SPI 0
  │    │    └─ [캐시 미스] parser::parse(query)        SPI 0   ← 2차 파싱 (같은 문자열을 두 번 판다)
  │    │         └─ Compiler::compile_read()          §2 참고
  │    └─ exec_json(sql, params)       mod.rs:145      SPI 1   ← 진짜 질의
  │         └─ 결과 전체를 Vec<serde_json::Value> 로 물질화
  └─ audit(...)                        mod.rs:122      SPI 1   ← og_audit INSERT (읽기 질의도 쓴다)
```

두 가지가 이 그림에서 바로 보인다.

- **읽기 질의도 매번 한 행을 쓴다.** `audit()` 은 `og_data.og_audit` 에 `INSERT` 한다
  ([`cypher/mod.rs:124-135`](../../../engine/src/cypher/mod.rs)). 실패는 무시되지만(`.ok()`),
  프라이머리에서는 실제 쓰기이고 WAL을 만든다 → [`PERF-13`](07_improvements_performance.md).
- **결과가 스트리밍되지 않는다.** `exec_json` 은 `.collect()` 로 모든 행을
  `Vec<serde_json::Value>` 에 담고 나서 `SetOfIterator` 를 만든다
  ([`cypher/mod.rs:145-152`](../../../engine/src/cypher/mod.rs), [`:108`](../../../engine/src/cypher/mod.rs)).
  행마다 jsonb Datum → `serde_json::Value` 역직렬화 → 다시 `JsonB` 재직렬화가 일어난다
  → [`PERF-12`](07_improvements_performance.md).

## 2. 사실 — 컴파일 경로의 SPI 왕복 (콜드 백엔드)

`MATCH (a:P)-[:K]->(b:P) WHERE a.val = 7 RETURN count(b)` 를 컴파일하는 동안의 SPI 호출을
코드 순서대로 센 결과다. 타입 뷰 `og_data.v_2` 는 **이미 존재한다**고 가정한다.

| # | 호출 | 코드 | SQL |
|---|---|---|---|
| 1 | `Compiler::new` → `types::graph_id` | [`compile.rs:155`](../../../engine/src/cypher/compile.rs) | `SELECT graph_id FROM og_catalog.graph WHERE name = $1` |
| 2 | `bind_node(a)` → `resolve_label_set` → `try_type_id("P")` | [`types.rs:158`](../../../engine/src/catalog/types.rs) | `SELECT type_id FROM og_catalog.type WHERE graph_id=$1 AND name=$2` |
| 3 | `views::ensure_view(P)` → `view_exists` | [`views.rs:95`](../../../engine/src/cypher/views.rs) | `SELECT true FROM pg_class …` |
| 4 | `bind_node(b)` → `try_type_id("P")` | 위와 같음 | 동일 질의 **재실행** |
| 5 | `views::ensure_view(P)` → `view_exists` | 위와 같음 | 동일 질의 **재실행** |
| 6 | `join_rel` → `types::try_type_id("K")` | [`compile.rs:835`](../../../engine/src/cypher/compile.rs) | 타입 조회 |
| 7 | `join_rel` → `labeling::og_subtypes(K)` | [`compile.rs:836`](../../../engine/src/cypher/compile.rs) | `type_label` 자기조인 |
| 8 | `count(b)` → `var_value` → `node_json` → `view_properties(P)` → `og_subtypes` | [`views.rs:34`](../../../engine/src/cypher/views.rs) | `type_label` 자기조인 |
| 9 | `view_properties(P)` → 프로퍼티 조회 | [`views.rs:39-43`](../../../engine/src/cypher/views.rs) | `SELECT name, column_name, data_type FROM og_catalog.property WHERE type_id = ANY($1)` |

**컴파일 = SPI 9회** (전부 카탈로그 조회, 캐시 없음, 2·4 와 3·5 는 완전히 같은 질의의 중복 실행).
여기에 실행(1) + 감사(1)를 더하면 **콜드 첫 호출 = SPI 11회**, 웜 호출 = **SPI 2회**.

타입 뷰가 아직 없으면(스키마 변경 직후에는 항상 그렇다 — §5) `ensure_view` 하나가 추가로
`og_subtypes`(1) + 프로퍼티 조회(1) + `og_subtypes`(1) + 저장 테이블 조회(1) +
구체 테이블마다 `own_columns`(1) + `CREATE OR REPLACE VIEW`(1) = **최소 6회**를 더 쓴다
([`views.rs:93-138`](../../../engine/src/cypher/views.rs)).

> 이것이 [`02_measured_baselines.md` §8](02_measured_baselines.md) 의
> "데이터 크기와 무관한 ~1,170 페이지"의 유력한 후보다. 확인 방법은 [`PERF-14`](07_improvements_performance.md).

## 3. 사실 — 실행 시점의 페이지 접근 (한 홉)

§1의 SQL이 실행될 때, 시작 노드 1개·차수 *d* 기준:

| 단계 | 접근 대상 | 인덱스 프로브 | 힙 페이지 |
|---|---|---|---|
| 라벨 스캔 | `og_data.v_2` → `og_data.n_2` | `WHERE a.val = 7` 의 계획에 달림 (§4) | 인덱스 스캔이면 ~1, 시퀀셜 스캔이면 테이블 전체 |
| 인접 세그먼트 | `og_adj` (PK `(src, etype, dir, seq)`) | **1** | `⌈d/256⌉` |
| 언네스트 | `unnest(nbr, eid)` | 0 | 0 (메모리) — 단 배열 **2개**를 디폼 |
| 도착 노드 조인 | `og_data.n_2` PK | **`d`** | **`d`** |
| 프로젝션 | `og_type_name(n2.type_id)` (SQL 함수, 인라인 가능) | 행마다 `og_catalog.type` PK 1회 | 행마다 1 |

**차수에 비례하는 항이 두 개 남아 있다** — 도착 노드 조인과 `og_type_name`.
`og_adj` 가 없앤 것은 "엣지 찾기"뿐이고 "도착 노드 얻기"는 AGE와 동일하다
([`01_performance_model.md` §3](01_performance_model.md)).

`og_type_name(n2.type_id)` 은 `(a:P)` 처럼 라벨이 컴파일 시점에 이미 하나의 타입으로 확정된
경우에도 방출된다 ([`compile.rs:1101`](../../../engine/src/cypher/compile.rs)) —
그 값이 컴파일 시점 상수인데도 그렇다. → [`PERF-05`](07_improvements_performance.md).

## 4. 사실 — 라벨 스캔이 실제로 만드는 계획

브리핑이 우려한 `type_id IN (SELECT og_subtype_ids(...))` 형태는 **Cypher 계획에 등장하지 않는다.**
그 형태는 [`access.sql:53-65`](../../../engine/sql/access.sql) 의 `og_nodes` / `og_edges` 안에만 있고,
`engine/src/` 안에 이 두 함수의 호출자는 없다(`grep` 확인).

Cypher의 라벨 스캔은 항상 **구체 테이블들의 `UNION ALL` 뷰**다
([`views.rs:102-135`](../../../engine/src/cypher/views.rs)):

```sql
CREATE OR REPLACE VIEW og_data.v_1 AS
  SELECT id, 1::int4 AS type_id, p_model, p_year, NULL::int4 AS p_range_km, __ext FROM og_data.n_1
  UNION ALL
  SELECT id, 4::int4,            p_model, p_year, p_range_km,             __ext FROM og_data.n_4
```

- **좋은 점**: 각 브랜치가 실제 테이블이므로 플래너가 브랜치별 통계·인덱스를 쓸 수 있고,
  서브타입 판정에 런타임 비용이 0이다.
- **주의할 점**: 서브타입이 없는 프로퍼티는 `NULL::type AS col` 상수로 채워진다
  ([`views.rs:114`](../../../engine/src/cypher/views.rs)).
  그 컬럼에 대한 조건은 해당 브랜치에서 항상 거짓이지만, 브랜치 자체는 계획에 남는다.
- **주의할 점**: 하위 타입이 늘어날수록 `Append` 의 브랜치 수가 선형으로 늘고,
  `MATCH (v:Vehicle)` 하나가 N개의 스캔이 된다. 큰 온톨로지에서의 동작은 **미확인**이다.

`WHERE a.val = 7` 이 `n1.p_val IS NOT DISTINCT FROM 7` 로 내려간다는 사실
([`compile.rs:1377`](../../../engine/src/cypher/compile.rs))은
바로 이 스캔의 계획을 결정한다 → [`PERF-01`](07_improvements_performance.md),
[`05_planner_interaction.md` §4](05_planner_interaction.md).

## 5. 사실 — 캐시가 없는 곳, 그리고 무효화되지 않는 캐시

| 대상 | 캐시 | 무효화 |
|---|---|---|
| 타입/그래프 카탈로그 조회 (`graph_id`, `try_type_id`, `type_kind`, `storage_table`, `og_subtypes`, `og_supertypes`, `view_properties`) | **없음.** 매번 SPI | — |
| 컴파일된 SQL 텍스트 | `PLAN_CACHE`, 백엔드-로컬, 512개 초과 시 **전체 삭제** ([`cypher/mod.rs:59-65`](../../../engine/src/cypher/mod.rs)) | **없음** |
| 타입 뷰 `og_data.v_*` / `ve_*` | 카탈로그에 실물 뷰로 존재 | 스키마 변경 시 **전부 DROP** ([`labeling.rs:172-182`](../../../engine/src/catalog/labeling.rs) → [`views.rs:159-177`](../../../engine/src/cypher/views.rs)) |
| PostgreSQL 실행 계획 | SPI에 텍스트를 넘기므로 재사용되지 않는 것으로 **추정** (`og_reach` 만 `client.prepare()` 사용) | — |

**여기에 결함이 하나 있다.** `bump_schema_version` 은 모든 타입 뷰를 DROP 하지만
`PLAN_CACHE` 는 건드리지 않는다. 캐시된 SQL 텍스트는 `og_data.v_2` 를 이름으로 참조하고,
캐시 히트 시에는 `ensure_view` 가 호출되지 않으므로 뷰를 다시 만들 기회가 없다.
`og_catalog.schema_version` 테이블은 주석상 정확히 이 용도("agents cache the schema;
this is their invalidation key" — [`bootstrap.sql:174-176`](../../../engine/sql/bootstrap.sql))로
존재하는데 `PLAN_CACHE` 는 그것을 읽지 않는다 → [`PERF-06`](07_improvements_performance.md).

## 6. 사실 — 쓰기 핫패스: 노드 1개를 만드는 데 드는 SPI

`SELECT og_create_node('g','P','{"name":"x"}')` — **`name` 이 이미 선언된 프로퍼티인 경우**:

| # | 호출 | 코드 |
|---|---|---|
| 1 | `types::graph_id` | [`types.rs:112-119`](../../../engine/src/catalog/types.rs) |
| 2 | `types::type_id` → `try_type_id` | [`types.rs:121-127`](../../../engine/src/catalog/types.rs) |
| 3 | `types::type_kind` | [`types.rs:286-294`](../../../engine/src/catalog/types.rs) |
| 4 | `types::storage_table` | [`types.rs:616-622`](../../../engine/src/catalog/types.rs) |
| 5 | `alloc_id` → `og_id_alloc` `INSERT … ON CONFLICT DO UPDATE … RETURNING` | [`storage/mod.rs:24-34`](../../../engine/src/storage/mod.rs) |
| 6 | `plan_props` → `og_catalog.property` 조회 | [`storage/mod.rs:161-177`](../../../engine/src/storage/mod.rs) |
| 7 | `declare_new_props` → 그래프/타입 이름 조회 | [`storage/mod.rs:90-100`](../../../engine/src/storage/mod.rs) |
| 8 | `INSERT INTO og_data.og_node` | [`storage/mod.rs:271-275`](../../../engine/src/storage/mod.rs) |
| 9 | `INSERT INTO og_data.n_<tid>` | [`storage/mod.rs:285`](../../../engine/src/storage/mod.rs) |

### **노드 1개 = SPI 9회.** 이 중 1~4·6·7 의 **6회는 순수 카탈로그 조회**이고, 실제 데이터 쓰기는 2회다.

Cypher `CREATE (n:P {name:'x'})` 로 들어오면:
- `run_write` 의 `graph_id` 1회 ([`cypher/mod.rs:168`](../../../engine/src/cypher/mod.rs))
- `resolve_or_create_label_set` → `try_type_id` + `resolve_label_set`→`try_type_id` = 2회
  ([`types.rs:207-221`](../../../engine/src/catalog/types.rs))
- `create_node_inner` = 7회 (위 3~9)
- 변수를 바인딩하면 `node_json(id)` 1회 ([`cypher/mod.rs:655-661`](../../../engine/src/cypher/mod.rs)),
  그 안의 `og_node_json` 은 plpgsql로 질의 2개 + 동적 `EXECUTE` 1개를 더 돈다
- 마지막에 `audit` 1회

→ **Cypher로 노드 1개 = SPI 11~12회.**

## 7. 사실 — 쓰기 핫패스: 엣지 1개

`create_edge_inner` ([`storage/mod.rs:402-452`](../../../engine/src/storage/mod.rs)):

| # | 호출 | 코드 |
|---|---|---|
| 1 | `types::type_kind` | `storage/mod.rs:410` |
| 2 | `types::storage_table` | `storage/mod.rs:413` |
| 3 | `validate_roles` → `labeling::og_supertypes(tid)` | `storage/mod.rs:462` |
| 4 | `validate_roles` → `og_catalog.role` 조회 | `storage/mod.rs:456-473` |
| 5.. | 선언된 role마다 `labeling::og_is_subtype` | `storage/mod.rs:476` |
| — | `alloc_id` | `storage/mod.rs:418` |
| — | `plan_props` → 프로퍼티 조회 | `storage/mod.rs:419` |
| — | `declare_new_props` → 이름 조회 | `storage/mod.rs:90-100` |
| — | `INSERT INTO og_data.og_edge` | `storage/mod.rs:429-433` |
| — | `INSERT INTO og_data.e_<tid>` | `storage/mod.rs:441` |
| — | `adjacency::append(src, 'o')` | `storage/mod.rs:445` |
| — | `adjacency::append(dst, 'i')` | `storage/mod.rs:446` |

`adjacency::append` 는 [`adjacency.rs:19-44`](../../../engine/src/storage/adjacency.rs) 에서
**항상 `UPDATE` 를 먼저 시도하고, 갱신된 행이 없으면 `INSERT` 를 한 번 더 한다.**
따라서 append 1회는 SPI **1회 또는 2회**다.

### **엣지 1개 = SPI 10~14회** (role이 0개일 때 10~12, 양방향 세그먼트가 모두 새로 생기면 +2).

Cypher `CREATE (a)-[:K]->(b)` 로 들어오면 여기에 라벨 해석 2회 + `edge_json` 1회 + `audit` 1회가 더해진다.

## 8. 사실 — 인접 갱신의 쓰기 증폭

```sql
UPDATE og_data.og_adj a
   SET nbr = a.nbr || $4::int8, eid = a.eid || $5::int8, n = a.n + 1
 WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::"char"
   AND a.seq = (SELECT max(seq) FROM og_data.og_adj
                 WHERE src = $1 AND etype = $2 AND dir = $3::text::"char")
   AND a.n < $6
RETURNING a.seq
```
([`adjacency.rs:22-31`](../../../engine/src/storage/adjacency.rs))

- PostgreSQL의 `UPDATE` 는 MVCC상 **튜플 전체의 새 버전을 쓴다.**
  `nbr`/`eid` 는 `STORAGE MAIN` 이므로 TOAST 밖 인라인이고
  ([`bootstrap.sql:210-211`](../../../engine/sql/bootstrap.sql)), 세그먼트가 가득 찼을 때
  이웃 **1개** 추가는 약 **4 KB의 튜플 재작성 + 그만큼의 WAL** 을 만든다.
  세그먼트가 `n` 개 이웃을 가질 때까지 누적 쓰기량은 `Σ 16·i ≈ 8·n²` 바이트다(추정, `int8` 2개 기준).
  `n = 256` 이면 약 **524 KB를 써서 4 KB짜리 세그먼트 하나를 완성한다.**
- 게다가 `WHERE` 절 안에 `(SELECT max(seq) …)` 상관 서브질의가 있어서
  **append 1회마다 같은 인덱스를 두 번 탄다.**
- `fillfactor = 80` ([`bootstrap.sql:206`](../../../engine/sql/bootstrap.sql))이 HOT 업데이트 여지를 두지만,
  4 KB 튜플에 8 KB 페이지의 20% (약 1.6 KB)로는 같은 페이지에 새 버전이 들어가지 못한다(추정).
- 벌크 로드 경로가 따로 없다. `COPY` / `copy_in` 은 코드베이스 어디에도 없고
  (`grep -rn "COPY \|copy_in"` → 히트 없음), Cypher 쓰기는 바인딩 행마다
  Rust 루프에서 위 절차를 반복한다 ([`cypher/mod.rs:236-243`](../../../engine/src/cypher/mod.rs)).

→ [`PERF-09`](07_improvements_performance.md), [`PERF-10`](07_improvements_performance.md),
[`PERF-11`](07_improvements_performance.md).

## 9. 사실 — 미선언 프로퍼티를 처음 쓸 때 일어나는 일

Cypher 애플리케이션은 스키마를 선언하지 않으므로, `declare_new_props`
([`storage/mod.rs:87-158`](../../../engine/src/storage/mod.rs))가 쓰기 시점에 컬럼을 승격한다.
새 프로퍼티 하나에 대해 `og_add_property`
([`types.rs:511-599`](../../../engine/src/catalog/types.rs))가 하는 일:

1. `graph_id` + `type_id` 조회 (SPI 2)
2. `og_catalog.property` INSERT (SPI 1)
3. `og_subtypes` (SPI 1) — 그리고 서브타입마다:
   - `storage_table` 조회 (SPI 1)
   - `ALTER TABLE … ADD COLUMN` — **AccessExclusiveLock** (SPI 1)
   - `UPDATE {table} SET {col} = (__ext ->> '…')::{dtype}, __ext = __ext - '…' WHERE __ext ? '…'`
     — **테이블 전체 스캔 + 해당 행 전부 재작성** ([`types.rs:561-567`](../../../engine/src/catalog/types.rs)) (SPI 1)
   - 서브타입이면 `property` INSERT (SPI 1)
4. `og_subtypes` 다시 (SPI 1) — 서브타입마다 `storage_table` + `try_type_name` +
   `ensure_alias_view`(DROP VIEW + CREATE VIEW = SPI 2) (SPI 4)
5. `bump_schema_version` → `drop_all_views()` (뷰 목록 SELECT 1 + 뷰마다 DROP) + `schema_version` INSERT

**단일 타입 계층에서도 SPI 약 15~17회, DDL 4개 이상, 테이블 전체 스캔 1회**가
사용자 트랜잭션 안에서 일어난다. 그리고 `drop_all_views()` 가 **모든** 타입 뷰를 지우므로,
이 데이터베이스의 다른 모든 백엔드가 다음 Cypher 질의에서 뷰를 다시 만들어야 한다.
→ [`PERF-10`](07_improvements_performance.md).

## 10. 사실 — `og_reach` 의 레벨 루프

[`traverse.rs:80-161`](../../../engine/src/storage/traverse.rs).

```rust
// One SPI connection and one plan for the whole walk, not one per level.
let out = Spi::connect(|client| {
    let stmt = client.prepare(sql.as_str(), &[INT8ARRAYOID, INT4ARRAYOID])...;
    for depth in 1..=maxhop {
        let segments: Vec<Vec<Option<i64>>> = client
            .select(&stmt, None, &[frontier.clone().into(), etypes.clone().into()])
            ...
```

- **잘된 점**: SPI 연결과 계획을 루프 밖으로 뺐다. 문서는 이 변경이
  chain 100,000홉을 1,196 ms → 1,016 ms 로 줄였다고 기록한다
  ([`docs/deep-traversal.md`](../../deep-traversal.md)).
- **레벨당 SPI 실행 1회.** 프론티어가 얇고 깊이가 큰 그래프(사슬)에서는
  이것이 전체 비용이 된다: chain-1M 100,000홉에서 `og_reach` 1,015.9 ms 대
  `og_reach_sql` 154.5 ms — **재귀 CTE에 6.6배 진다**
  ([`bench/csr/results/deep-chain-20260817T053710Z.json`](../../../bench/csr/results/deep-chain-20260817T053710Z.json)).
- **레벨마다 `frontier.clone()` 과 `etypes.clone()`** ([`traverse.rs:136`](../../../engine/src/storage/traverse.rs)).
  프론티어는 최대 `|V|` 크기이므로 dense 픽스처에서 레벨당 최대 50,000×8 B = 400 KB의 복사가 추가된다(추정).
- **`Vec<Vec<Option<i64>>>`** 로 세그먼트를 받는다 ([`traverse.rs:135-139`](../../../engine/src/storage/traverse.rs)).
  세그먼트마다 `Vec` 할당 1회, 이웃마다 `Option<i64>`(16 B). 999,784 엣지를 한 번 훑으면
  약 16 MB의 임시 할당이다(추정).
- **`HashSet<i64>`** 는 표준 SipHash를 쓴다 ([`traverse.rs:118`](../../../engine/src/storage/traverse.rs)).
  같은 파일의 CSR 경로는 같은 일을 비트맵으로 한다 ([`traverse.rs:346-353`](../../../engine/src/storage/traverse.rs)).
- **결과 전체를 `Vec<(i64,i32)>` 에 담고서야 반환한다** ([`traverse.rs:157-160`](../../../engine/src/storage/traverse.rs)).

→ [`PERF-04`](07_improvements_performance.md), [`PERF-15`](07_improvements_performance.md).

## 11. 사실 — 결과 전달 경로 (Bolt / Studio)

### Bolt 게이트웨이 — RUN 1회당 PostgreSQL 왕복 3회

[`bolt/src/session.rs:242-329`](../../../bolt/src/session.rs):

| 순서 | 질의 | 하는 일 |
|---|---|---|
| 1 | `SELECT og_cypher_check($1::text)::text` (`is_write`, [`session.rs:444-461`](../../../bolt/src/session.rs)) | 질의를 **파싱** |
| 2 | `SELECT og_cypher_columns($1::text)` ([`session.rs:283-289`](../../../bolt/src/session.rs)) | 질의를 **다시 파싱** |
| 3 | `SELECT og_cypher($1,$2,$3::jsonb)::text` ([`session.rs:291-299`](../../../bolt/src/session.rs)) | 질의를 **또 파싱** + 실행 |

`og_cypher` 내부에서도 파싱이 1회 더 있고([`cypher/mod.rs:93`](../../../engine/src/cypher/mod.rs)),
`PLAN_CACHE` 미스면 `compile_cached` 가 또 한 번 판다([`cypher/mod.rs:52`](../../../engine/src/cypher/mod.rs)).
→ **하나의 Bolt RUN에 대해 Cypher 렉싱·파싱이 4~5회.**

그리고 `::text` 캐스트가 붙어 있다. 표현 변환은 행마다 다음과 같이 일어난다:

```
PostgreSQL jsonb (바이너리) → text 직렬화 → 와이어 → serde_json::from_str
  → serde_json::Value → to_bolt() → packstream::Value → encode() → 바이트
```

전체 결과가 `self.pending: Vec<Value>` 에 물질화된 뒤
([`session.rs:319-320`](../../../bolt/src/session.rs)) PULL이 시작되고,
PULL은 레코드마다 `self.pending[…].clone()` 으로 **깊은 복사**를 한 뒤 쓴다
([`session.rs:353`](../../../bolt/src/session.rs)).
`write_message` 는 메시지마다 새 `Vec` 을 만들고 **`w.flush()` 를 호출**하는데
([`packstream.rs:254-262`](../../../bolt/src/packstream.rs)),
`w` 는 버퍼가 없는 `TcpStream` 이다 ([`session.rs:336`](../../../bolt/src/session.rs)) —
레코드 1개마다 최소 3회의 `write()` 시스템 콜이 발생한다.
→ [`PERF-16`](07_improvements_performance.md), [`PERF-17`](07_improvements_performance.md).

### Studio 서버 — 전체 결과를 메모리에 모은다

[`portal/server/index.js:182-215`](../../../portal/server/index.js):

1. `client.query('SELECT og_cypher($1,$2,$3) AS row', …)` — `pg` 의 기본 `query()` 는
   **커서를 쓰지 않고 모든 행을 배열로 버퍼링한다.**
2. `const rows = r.rows.map((x) => x.row)` — 두 번째 배열
3. `projectGraph(rows)` ([`index.js:317-342`](../../../portal/server/index.js)) —
   모든 행의 모든 값을 재귀 순회하며 노드·엣지 `Map` 두 개를 만든다
4. `json(res, 200, {...})` → `JSON.stringify(body)` + `content-length` 계산
   ([`index.js:39-45`](../../../portal/server/index.js)) — 전체를 문자열 하나로

**스트리밍은 없다. 행 수 상한도, `statement_timeout` 도 없다.**
`MATCH (n) RETURN n` 하나로 Node 프로세스가 죽을 수 있다.
게다가 같은 요청에서 `og_cypher_sql` 을 한 번 더 호출해 컴파일을 다시 돌린다
([`index.js:197-202`](../../../portal/server/index.js)). → [`PERF-18`](07_improvements_performance.md).

<!-- affects: backend, api, frontend -->
<!-- requires-update: docs/01_architecture/09_performance/07_improvements_performance.md -->
