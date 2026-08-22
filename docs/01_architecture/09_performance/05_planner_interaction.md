# 플래너 상호작용 — PostgreSQL은 이 SQL을 어떻게 보는가

> **이 문서가 답하는 질문**
> - 플래너가 실제로 볼 수 있는 것은 어디까지이고, 볼 수 없는 것은 어디부터인가?
> - 통계는 어디에서 오는가?
> - `access.sql` 의 `ROWS` 고정 추정치(50/500/1000/8/100)는 실제와 얼마나 다른가?
> - 조인 순서와 병렬화는 무엇에 의해 결정되는가?
> - 오추정은 정확히 어디에서 생기는가?

---

## 1. 사실 — 플래너의 시야 경계

이 프로젝트의 핵심 주장은 "AGE와 달리 플래너가 패턴을 본다"이다
([`compile.rs:1-10`](../../../engine/src/cypher/compile.rs)). 정확한 범위는 다음과 같다.

```
SELECT og_cypher('g', 'MATCH …')            ← 최상위 문장. 플래너는 함수 호출 하나만 본다.
   └─ SPI: SELECT jsonb_build_object(…) FROM og_data.v_2 n1 CROSS JOIN LATERAL (…) …
                                             ← 이 안쪽은 진짜 질의 트리다. 플래너가 전부 본다.
```

**본다**: 라벨 스캔의 구체 테이블들, 컬럼 통계, 인덱스, `og_adj` 스캔, 조인 순서, 집계.
**보지 못한다**: 최상위 문장에서 `og_cypher()` 가 무슨 행을 몇 개 낼지.
그리고 `og_vlp` / `og_reach` 의 안쪽 — 전자는 CTE라 인라인되지 않고, 후자는 Rust SRF다.

> AGE와의 차이는 "함수 안을 보느냐"가 아니라 **"프로퍼티가 실컬럼이라 통계와 인덱스가 있느냐"** 다.
> [`docs/comparison.md`](../../comparison.md) 가 이 정정을 이미 기록하고 있다:
> *"it is not true that the planner sees nothing"*.

## 2. 사실 — 통계의 출처

| 통계 | 출처 | 누가 쓰는가 |
|---|---|---|
| 노드/엣지 컬럼 분포 | `og_data.n_*` / `e_*` 의 일반 `pg_statistic` (ANALYZE) | 라벨 스캔의 선택도, 조인 순서 |
| 타입 계층 | `og_catalog.type_label` 의 `(graph_id, lft, rgt)` 인덱스 | **컴파일 시점** 라벨 해석 ([`views.rs:14-17`](../../../engine/src/cypher/views.rs)) — 런타임 통계가 아니다 |
| 그래프 전체 노드/엣지 수 | `pg_class.reltuples` | `prefer_reachability` ([`compile.rs:46-50`](../../../engine/src/cypher/compile.rs)) — **컴파일 시점** |
| 인접 세그먼트의 배열 길이 | `og_adj.n` 컬럼에 통계는 있으나, 플래너는 이것을 `unnest(nbr)` 의 카디널리티와 연결하지 않는다 | 아무도 — §4.3 |
| 차수 분포 | `og_degree_distribution()` ([`storage/stats.rs:86-115`](../../../engine/src/storage/stats.rs)) | 운영자·에이전트용. **플래너는 읽지 않는다** |

**핵심**: `og_degree`, `og_degree_all`, `og_degree_distribution`, `og_graph_stats` 는
모두 "spec 001 FR-015: Cypher 플래너가 확장 순서를 고르는 데 쓰는 통계"라고 주석되어 있으나
([`adjacency.rs:74-76`](../../../engine/src/storage/adjacency.rs)),
**`engine/src/cypher/` 안에 이 함수들의 호출자는 없다**(`grep` 확인).
확장 순서를 고르는 것은 PostgreSQL의 플래너이고, 그것은 이 통계들을 모른다.

## 3. 사실 — `ROWS` 고정 추정치와 실제

[`engine/sql/access.sql`](../../../engine/sql/access.sql) 에 선언된 값:

| 함수 | 라인 | 선언 `ROWS` | 벤치 픽스처에서의 실제 |
|---|---|---|---|
| `og_expand` | `:16` | **50** | 5k 픽스처 8, 50k 픽스처 20, chain 1, grid 2 |
| `og_expand_batch` | `:31` | **500** | 시작점 수 × 위 값 |
| `og_subtype_ids` | `:45` | **8** | 벤치 그래프는 타입 2개. 큰 온톨로지에서는 수백 가능 |
| `og_nodes` | `:55` | **1000** | 50k 픽스처 50,000 |
| `og_edges` | `:62` | **1000** | 50k 픽스처 974,936 |
| `og_vlp` | `:140` | **100** | 깊이 3·차수 20에서 8,420 / 깊이 6에서 약 6,700만 |
| `og_reach_sql` | `:171` | **1000** | `k × |V|` 까지 |
| `og_reach` | `:197` (`ALTER FUNCTION … ROWS 100`) | **100** | `|V|` 까지 — 50k 픽스처에서 50,000 |

### 3.1 어느 것이 실제로 계획에 영향을 주는가

`LANGUAGE sql` 집합반환함수의 `ROWS` 는 **인라인되지 않은 경우에만** 쓰인다.
인라인되면 플래너가 본문 자체의 추정을 쓴다.

| 함수 | 인라인 | `ROWS` 가 계획에 영향? | Cypher 핫패스에 있는가 |
|---|---|---|---|
| `og_expand`, `og_expand_batch`, `og_subtype_ids` | 가능(단일 SELECT) | 아니오(인라인되면 무시) | **아니오** |
| `og_nodes`, `og_edges` | 서브링크 포함 — 미확인 | 미확인 | **아니오** (호출자 없음) |
| `og_vlp` | **불가로 추정**(`WITH RECURSIVE`) | **예** | **예** |
| `og_reach_sql` | **불가로 추정**(`WITH RECURSIVE`) | 예 | 아니오 |
| `og_reach` | Rust SRF — 절대 인라인 안 됨 | **예** | **예** |

**결론: 계획에 실제로 영향을 주는 두 개(`og_vlp` 100, `og_reach` 100)가 하필 가장 크게 빗나간다.**
깊이 6, 차수 20에서 `og_vlp` 는 100행으로 추정되지만 실제로는 약 6,700만 행을 낸다 — **약 67만 배**.
`og_reach` 는 100행으로 추정되지만 포화 후 50,000행을 낸다 — **500배**.

### 3.2 두 값이 왜 100으로 맞춰져 있는가

의도적이다. [`access.sql:192-197`](../../../engine/sql/access.sql):

```sql
-- `og_reach` is written in Rust, and pgrx gives every set-returning function
-- PostgreSQL's default guess of 1000 rows. `og_vlp` declares 100. Two functions
-- that answer the same question must not be costed an order of magnitude apart
-- for a reason that has nothing to do with either — the planner would pick
-- different join orders for the two and the comparison would measure the guess.
ALTER FUNCTION og_reach(int8, int4[], "char", int4, int4) ROWS 100;
```

**두 경로를 서로 비교 가능하게 만든 것은 옳다. 그러나 두 값 다 실제와 맞지 않는다.**
그 결과 두 경로 모두 상위 조인(`n2.id = w.node`)의 크기를 크게 과소평가하고,
플래너는 50,000행짜리 입력에 대해 100행용 계획(중첩 루프)을 고른다.
→ [`PERF-21`](07_improvements_performance.md).

## 4. 사실 — 오추정이 생기는 지점 (하나씩)

### 4.1 `WHERE a.val = 7` → `IS NOT DISTINCT FROM`

[`compile.rs:1377`](../../../engine/src/cypher/compile.rs):

```rust
BinOp::Eq => format!("({ls} IS NOT DISTINCT FROM {rs})"),
BinOp::Ne => format!("({ls} IS DISTINCT FROM {rs})"),
```

이것은 PostgreSQL의 파스 트리에서 `OpExpr` 가 아니라 `DistinctExpr` 다.
PostgreSQL의 인덱스 경로 생성기(`match_clause_to_indexcol`)는
`OpExpr` / `ScalarArrayOpExpr` / `NullTest` / `RowCompareExpr` 만 인덱스 조건으로 매칭하며,
`DistinctExpr` 은 그 목록에 없다. **즉 `og_create_index` 로 만든 B-tree를 쓸 수 없다.**

같은 조건을 인라인 맵으로 쓰면 다른 SQL이 나온다
([`compile.rs:812`](../../../engine/src/cypher/compile.rs)):

```rust
self.constrain(format!("{lhs} = {rhs}"));   // (a:P {val: 7}) → n1.p_val = 7 — 인덱스 사용 가능
```

**그리고 벤치 하네스는 인덱스를 못 쓰는 쪽을 쓴다**
([`bench/harness.py:361-373`](../../../bench/harness.py) 의 `WHERE a.val = {start_local}`),
반면 [`bench/csr/cypher_ab.sql`](../../../bench/csr/cypher_ab.sql) 은 `{val:7}` 인라인 맵을 쓴다.
같은 저장소의 두 벤치가 서로 다른 접근 경로를 재고 있다.

**신뢰도**: SQL이 생성되는 것은 사실(코드). 실제 계획은 확인하지 못했다.
확인 방법은 [`PERF-01`](07_improvements_performance.md).

### 4.2 타입 뷰의 `UNION ALL` 과 상수 `NULL` 컬럼

[`views.rs:110-116`](../../../engine/src/cypher/views.rs) 은 서브타입이 갖지 않은 프로퍼티를
`NULL::{dtype} AS {col}` 로 채운다. 그 컬럼에 대한 조건은 해당 브랜치에서 통계가 없는
상수 표현식이 되고, 플래너는 기본 선택도를 쓴다.
서브타입 수가 늘면 `Append` 브랜치가 선형으로 늘어난다. **큰 계층에서의 동작은 미확인.**

### 4.3 `unnest(adj.nbr, adj.eid)` 의 카디널리티

플래너에게 `unnest()` 의 출력 행 수를 알려 주는 것은 함수의 `prorows` 기본값(100)이다.
`og_adj.n` 에 실제 배열 길이가 들어 있지만 플래너는 그것을 참조하지 않는다.
평균 차수 20인 픽스처에서는 5배 과대추정, 사슬(차수 1)에서는 100배 과대추정이다.

**신뢰도**: `prorows` 기본값이 100이라는 것은 PostgreSQL의 카탈로그 사실이나,
`unnest` 에 배열 통계를 쓰는 특수 처리가 있는지는 **미확인**.
확인 방법: `EXPLAIN` 의 `Function Scan on unnest` 노드의 `rows=` 값을 본다.

### 4.4 도착 노드 조인의 크기 추정

`WHERE n2.id = u4.nbr` 에서 `u4.nbr` 은 함수/`unnest` 출력이므로 통계가 없다.
플래너는 `n2` 의 행 수와 기본 조인 선택도로 결과 크기를 추정한다.
`og_reach` 출력과 조인할 때는 §3.2의 `ROWS 100` 이 곱해져 **50,000행짜리 조인이 100행으로 계획된다**(추정).
이것이 [`02_measured_baselines.md` §4](02_measured_baselines.md) 의
"4홉에서 195,202 대 16,170 페이지" 차이를 만드는 유력한 경로다.

### 4.5 `prefer_reachability` 의 통계 범위

[`compile.rs:46-58`](../../../engine/src/cypher/compile.rs) 은
`og_data.og_node` / `og_data.og_edge` 의 `reltuples` 를 읽는다. 이것은
**데이터베이스 전체**의 레지스트리다. 다음 경우에 판정이 틀린다.

- 한 데이터베이스에 여러 그래프가 있을 때 (`og_create_graph` 는 여러 그래프를 허용한다).
- 관계 타입별 차수 편차가 클 때 — 평균은 하나뿐이다.
- 방향별 차수가 다를 때 — `edges/nodes` 는 방향을 구분하지 않는다.
- `ANALYZE` 되지 않았을 때 — `reltuples ≤ 0` 이면 `max >= 4` 로 되돌아간다
  ([`compile.rs:53`](../../../engine/src/cypher/compile.rs)).

### 4.6 `count(DISTINCT b)` 의 비교 비용

`b` 가 노드일 때 `var_value` 는 노드 전체의 jsonb를 만든다
([`compile.rs:1095-1113`](../../../engine/src/cypher/compile.rs)).
플래너는 `count(DISTINCT …)` 를 정렬 기반으로 계획하는데,
정렬 키가 **jsonb** 이므로 비교 비용이 `int8` 보다 훨씬 크고, `work_mem` 을 넘기면 디스크 정렬이 된다.
기본 `work_mem = 4 MB` (튜닝 없음 — [`02_measured_baselines.md` §1](02_measured_baselines.md))에서
50,000개의 노드 jsonb는 넘길 가능성이 높다(추정). → [`PERF-02`](07_improvements_performance.md).

## 5. 사실 — 조인 순서

컴파일러가 `FROM` 에 넣는 순서
([`compile.rs:662-684`](../../../engine/src/cypher/compile.rs), `move_join_to_end` 포함):

```
og_data.v_2 n1                                  ← 첫 노드는 항상 평범한 스캔
CROSS JOIN LATERAL ( og_adj … unnest … ) u4     ← 홉
CROSS JOIN og_data.v_2 n2                       ← 도착 노드 (조인 조건은 WHERE 로)
```

- **`LATERAL` 이 순서를 강제한다.** 인접 스캔은 반드시 `n1` 뒤에 온다.
  이것이 의도다 — 그러지 않으면 `og_adj` 전체를 훑게 된다.
- **도착 노드 조인은 강제되지 않는다.** `n2.id = u4.nbr` 는 `WHERE` 에 있는 평범한 등가 조인이므로
  플래너가 중첩 루프 / 해시 / 머지 중에서 고르고, 어느 쪽을 앞에 둘지도 고른다.
  §4.4의 오추정이 여기에 그대로 작용한다.
- **`OPTIONAL MATCH` 는 다르다.** 술어가 `WHERE` 가 아니라 `ON` 으로 들어가고
  ([`compile.rs:252-275`](../../../engine/src/cypher/compile.rs)),
  `LEFT JOIN` 은 재정렬 자유도를 크게 줄인다.
- **`WITH` 는 지평선(horizon)** 이다. 앞부분 전체가 서브질의가 되고
  모든 컬럼이 `to_jsonb` 로 감싸진다 ([`compile.rs:629-633`](../../../engine/src/cypher/compile.rs)).
  타입 정보가 사라지므로 그 뒤의 비교는 인덱스를 쓸 수 없다.

## 6. 사실 — 병렬화

| 대상 | 선언 | 결과 |
|---|---|---|
| `og_cypher` | `#[pg_extern]` — **parallel 속성 없음** ([`cypher/mod.rs:83`](../../../engine/src/cypher/mod.rs)) | PostgreSQL 기본값 `PARALLEL UNSAFE`. **`SELECT og_cypher(…)` 를 포함한 계획은 절대 병렬화되지 않는다** |
| `og_reach`, `og_csr_reach`, `og_csr_hops` | `parallel_restricted` ([`traverse.rs:80,359,442`](../../../engine/src/storage/traverse.rs)) | 리더에서만 실행 |
| `access.sql` 함수 9개 | `PARALLEL SAFE` | 병렬 가능 |
| `og_subtypes` / `og_supertypes` / `og_is_subtype` | `parallel_safe` ([`labeling.rs:192,212,232`](../../../engine/src/catalog/labeling.rs)) | 병렬 가능 — 단 내부에서 SPI를 쓴다 |
| `og_id_type` 등 4개 | `immutable, parallel_safe` ([`id.rs:73-91`](../../../engine/src/id.rs)) | 순수 산술. 병렬·인덱스 표현식 모두 가능 |
| **나머지** | `#[pg_extern]` 78개 중 **61개가 parallel 속성 없음** (`grep` 집계) | 전부 `PARALLEL UNSAFE` |

두 가지를 짚어야 한다.

1. **`og_cypher` 가 `PARALLEL UNSAFE` 라는 사실은 최상위 문장을 확실히 직렬화한다.**
   Cypher로 들어오는 모든 읽기가 그렇다.
2. **컴파일된 안쪽 SQL이 병렬 계획을 받을 수 있는지는 미확인이다.**
   SPI를 통해 실행되는 질의에 PostgreSQL이 `CURSOR_OPT_PARALLEL_OK` 를 부여하는지 확인하지 않았다.
   확인 방법: `SELECT og_cypher_explain('g', '<큰 스캔 질의>')` 의 `plan` 에 `Gather` 노드가 있는지 본다
   ([`cypher/mod.rs:677-696`](../../../engine/src/cypher/mod.rs) — 이 함수도 SPI로 `EXPLAIN` 한다).
   → [`PERF-24`](07_improvements_performance.md).

`og_is_subtype` 이 `parallel_safe` 로 선언되어 있지만 내부에서 SPI를 호출한다는 점
([`labeling.rs:232-244`](../../../engine/src/catalog/labeling.rs))도 기록해 둔다.
SPI를 사용하는 함수는 일반적으로 `PARALLEL RESTRICTED` 여야 한다 — **정확성 위험이며 미확인**이다.

## 7. 사실 — 사용할 수 있는 진단 도구

| 함수 | 하는 일 | 코드 |
|---|---|---|
| `og_cypher_sql(graph, query)` | 컴파일된 SQL 텍스트를 그대로 돌려준다. `EXPLAIN` 에 붙여넣을 수 있다 | [`cypher/mod.rs:74-80`](../../../engine/src/cypher/mod.rs) |
| `og_cypher_explain(graph, query, analyze)` | 컴파일된 SQL을 `EXPLAIN (FORMAT JSON)` (또는 `ANALYZE, BUFFERS`) 한다 | [`cypher/mod.rs:677-696`](../../../engine/src/cypher/mod.rs) |
| `og_estimate(graph, query)` | `Plan Rows` / `Total Cost` 를 뽑아 조언을 붙인다 | [`agent/mod.rs:351-395`](../../../engine/src/agent/mod.rs) |
| `og_graph_stats` / `og_degree_distribution` | 실제 차수 분포 (플래너와는 무관) | [`storage/stats.rs:12-115`](../../../engine/src/storage/stats.rs) |

**권장 절차** (개선 제안을 검증할 때는 항상 이 순서):

```sql
-- 1. 무엇이 만들어지는지 본다
SELECT og_cypher_sql('benchg',
  $$MATCH (a:P)-[:K*1..4]->(b:P) WHERE a.val = 7 RETURN count(DISTINCT b)$$);

-- 2. 그 SQL을 그대로 EXPLAIN 한다 ($1 은 jsonb 파라미터)
EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT)
  <위 출력>;                       -- $1 자리에 '{}'::jsonb 를 넣는다

-- 3. 함수 호출을 통과했을 때와 비교한다
SELECT og_cypher_explain('benchg',
  $$MATCH (a:P)-[:K*1..4]->(b:P) WHERE a.val = 7 RETURN count(DISTINCT b)$$, true);
```

## 8. 규칙

**필수**

- ✅ 계획에 대한 주장은 `og_cypher_sql` + `EXPLAIN` 출력을 근거로 한다.
- ✅ `ROWS` 를 바꾸기 전에 그 함수가 인라인되는지 먼저 확인한다(`EXPLAIN` 에 `Function Scan` 이 있는가).
- ✅ 벤치를 다시 돌릴 때 하네스의 `WHERE a.val = …` 표기를 바꾸지 않는다.
  바꿔야 한다면 베이스라인도 함께 갱신하고 그 사실을 기록한다
  ([`bench/README.md`](../../../bench/README.md) 의 "update it deliberately").

**금지**

- ❌ `og_degree*` 통계가 플래너에 반영된다고 쓰는 것. 반영되지 않는다.
- ❌ "함수가 인라인되므로 플래너가 전부 본다"를 `og_vlp` / `og_reach` / `og_node_json` 에 적용하는 것.
- ❌ 병렬 실행이 일어난다고 가정하는 것. 최상위 문장은 `PARALLEL UNSAFE` 함수 호출이다.

<!-- affects: backend, data -->
<!-- requires-update: docs/01_architecture/09_performance/07_improvements_performance.md -->
