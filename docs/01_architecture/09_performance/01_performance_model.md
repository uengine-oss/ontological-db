# 성능 모델 — 한 홉의 비용은 무엇으로 이루어지는가

> **이 문서가 답하는 질문**
> - 이 엔진에서 "한 홉"의 비용은 정확히 어떤 항목의 합인가?
> - Apache AGE 대비 우리가 실제로 없앤 비용은 무엇이고, **그대로 남아 있는** 비용은 무엇인가?
> - 홉이 깊어질 때 비용은 어떤 함수로 늘어나는가?
> - 이 모델에서 어디까지가 측정된 사실이고, 어디부터가 추정인가?

---

## 1. 사실 — 한 홉이 컴파일되는 실제 SQL

`MATCH (a:P)-[:K]->(b:P) WHERE a.val = 7 RETURN count(b)` 는
[`engine/src/cypher/compile.rs:900-906`](../../../engine/src/cypher/compile.rs) 이 만드는 다음 형태로 내려간다
(별칭 번호는 [`compile.rs:288-291`](../../../engine/src/cypher/compile.rs) 의 `fresh()` 규칙을 따른다).

```sql
SELECT jsonb_build_object('count(b)', t.c0) AS row FROM (
SELECT count(( jsonb_strip_nulls(jsonb_build_object(
                 '_id', n2.id, '_type', og_type_name(n2.type_id),
                 'name', to_jsonb(n2.p_name), 'val', to_jsonb(n2.p_val)))
             || COALESCE(n2.__ext,'{}'::jsonb) )) AS c0
  FROM og_data.v_2 n1
  CROSS JOIN LATERAL (SELECT u.nbr, u.eid
                        FROM og_data.og_adj adj3,
                             LATERAL unnest(adj3.nbr, adj3.eid) AS u(nbr, eid)
                       WHERE adj3.src = n1.id
                         AND adj3.dir = 'o'::"char"
                         AND adj3.etype = ANY(ARRAY[3]::int4[])) u4
  CROSS JOIN og_data.v_2 n2
 WHERE n2.id = u4.nbr
   AND (n1.p_val IS NOT DISTINCT FROM 7)
) t
```

세 가지를 이 SQL에서 바로 읽을 수 있다.

1. **라벨은 런타임 비용이 0이다.** `(a:P)` 는 `og_data.v_2` 라는 구체 테이블들의 `UNION ALL` 뷰로
   *컴파일 시점에* 확정된다 ([`engine/src/cypher/views.rs:93-138`](../../../engine/src/cypher/views.rs)).
   런타임에 계층을 걷는 코드는 없다.
2. **도착 노드 `n2` 는 별도 릴레이션으로 다시 조인된다.** `og_adj` 가 이웃 id를 이미 들고 있음에도,
   라벨 확인과 프로퍼티 접근을 위해 타입 뷰를 id로 재조회한다.
3. **`WHERE a.val = 7` 은 `=` 가 아니라 `IS NOT DISTINCT FROM` 으로 내려간다**
   ([`compile.rs:1377`](../../../engine/src/cypher/compile.rs)).
   같은 조건을 인라인 맵으로 쓴 `(a:P {val: 7})` 은 `push_prop_filters`
   ([`compile.rs:803-815`](../../../engine/src/cypher/compile.rs))를 지나 `n1.p_val = 7` 이 된다.
   두 표기가 서로 다른 SQL을 만든다는 것은 코드에서 확인한 사실이고,
   그것이 계획에 미치는 영향은 [`05_planner_interaction.md` §4](05_planner_interaction.md) 와
   [`PERF-01`](07_improvements_performance.md) 에서 다룬다.

## 2. 사실 — 한 홉의 비용 항목

시작 노드 하나, 해당 관계 타입·방향의 차수를 *d* 라 할 때, 위 SQL이 하는 일은 다음 다섯 가지다.

| 항목 | 무엇을 하는가 | 비용 | 근거 |
|---|---|---|---|
| **C1 라벨 해석** | `(a:P)` → 구체 테이블 목록 | 런타임 0 (컴파일 시 1회 구간 인덱스 범위 스캔) | [`views.rs:14-17`](../../../engine/src/cypher/views.rs), [`labeling.rs:192-209`](../../../engine/src/catalog/labeling.rs) |
| **C2 세그먼트 조회** | `og_adj` 에서 `(src, etype, dir)` 세그먼트 읽기 | PK 인덱스 프로브 1회 + 힙 튜플 `⌈d/256⌉` 개 | [`bootstrap.sql:197-211`](../../../engine/sql/bootstrap.sql) |
| **C3 언네스트** | `unnest(nbr, eid)` | `int8` 배열 **2개**를 디폼하여 `d` 행 생성 | [`compile.rs:901-903`](../../../engine/src/cypher/compile.rs) |
| **C4 도착 노드 재조인** | `n2.id = u.nbr` | `d` 회 PK 인덱스 프로브 + `d` 회 힙 페치 | [`compile.rs:906`](../../../engine/src/cypher/compile.rs) |
| **C5 프로젝션** | 행마다 노드 jsonb 조립 | `d` 회 `jsonb_build_object` + `og_type_name()` + `jsonb_strip_nulls` + `\|\|` | [`compile.rs:1095-1113`](../../../engine/src/cypher/compile.rs) |

`og_adj` 한 행은 이웃 최대 256개를 `int8[] × 2 = 4 KB` 로 들고 있고
`STORAGE MAIN` 으로 TOAST를 막아 놓았으므로 ([`bootstrap.sql:210-211`](../../../engine/sql/bootstrap.sql)),
C2는 차수와 거의 무관한 상수다. **C2가 이 설계의 핵심이다.**

## 3. 사실 — AGE 대비: 없앤 비용과 남은 비용

Apache AGE에서 한 홉은 대략 이렇다 ([`docs/comparison.md`](../../comparison.md) "Traversing"):

| 항목 | AGE | Ontological |
|---|---|---|
| 엣지 찾기 | `KNOWS.start_id` B-tree에 `d`회 하강 + `d`회 랜덤 힙 페치 | **인덱스 프로브 1회 + 힙 튜플 `⌈d/256⌉` 개** |
| 도착 노드 얻기 | `end_id` 로 `d`회 인덱스 조회 + `d`회 힙 페치 | `d`회 PK 프로브 + `d`회 힙 페치 — **동일** |
| 프로퍼티 읽기 | `agtype` JSON 파싱 | 실컬럼 읽기 |

즉 **우리가 구조적으로 없앤 것은 첫 줄뿐이다.** 두 번째 줄은 그대로 남아 있고,
1홉의 논리 페이지 접근이 두 시스템에서 거의 같은 이유가 이것이다.

50,000 노드 / 974,936 엣지, 1홉 논리 페이지 접근
([`bench/results/bench-50000-20260806T042833Z.json`](../../../bench/results/bench-50000-20260806T042833Z.json)):

| | Apache AGE | Ontological (Cypher) | Ontological (스토리지 경로) | 재귀 CTE |
|---|---|---|---|---|
| 1홉 페이지 | 1,707 | 1,742 | 389 | 8 |

README도 이 점을 명시한다 — *"a single indexed hop through AGE reads about as many pages as we do"*
([`README.md:41-42`](../../../README.md)).
**저장 구조의 이득은 실제로 존재하지만(389 대 1,707), Cypher 표면을 지나면 대부분 사라진다(1,742).**
그 차이가 어디에서 오는지는 [`03_hot_paths.md`](03_hot_paths.md) 에서 단계별로 센다.

## 4. 사실 — k홉으로 갈 때의 비용 함수

세 가지 서로 다른 함수가 있고, 어떤 것이 적용되는지는
[`04_deep_traversal_mechanics.md`](04_deep_traversal_mechanics.md) 의 전환 판정이 정한다.

| 경로 | 행 수 | 비용 함수 | 어디에 |
|---|---|---|---|
| 고정 길이 패턴 `k`개 연결 | `Σ` 각 홉의 `d` | `O(d^k)` (패턴이 실제로 그만큼 매치될 때) | `compile.rs:888-955` |
| `og_vlp` — 트레일 열거 | `Σ_{i=1..k} d^i` | `O(d^k)` — **질문의 형태에 비례** | [`access.sql:138-156`](../../../engine/sql/access.sql) |
| `og_reach` — 방문집합 BFS | `≤ |V|` | `O(|V| + |E|)` — **답의 크기에 비례** | [`traverse.rs:80-161`](../../../engine/src/storage/traverse.rs) |
| `og_csr_reach` — 컴파일 CSR | `≤ |V|` | `O(|V| + |E|)`, 단 힙·MVCC·플래너 없음 | [`traverse.rs:359-401`](../../../engine/src/storage/traverse.rs) |

`og_vlp` 의 곡선이 정확히 평균 차수라는 것은 측정으로 확인되어 있다:
dense 픽스처(50,000 노드 / 999,784 엣지, 평균 차수 20)에서
0.51 → 6.85 → 106.72 → 2,300 → 49,334 ms 로 홉당 13, 16, 22, 21배
([`docs/deep-traversal.md`](../../deep-traversal.md),
[`bench/csr/results/deep-dense-20260817T021522Z.json`](../../../bench/csr/results/deep-dense-20260817T021522Z.json)).

**중요한 비대칭:** `og_reach` 로 바뀌어도 C4·C5는 사라지지 않는다.
BFS는 노드 **id**만 돌려주지만, Cypher는 그 id마다 다시 타입 뷰를 조인하고 jsonb를 만든다.
50,000 노드 4홉에서 측정된 페이지 수가 그 값이다
([`bench/results/bench-50000-20260817T033001Z.json`](../../../bench/results/bench-50000-20260817T033001Z.json)):

| 깊이 | Cypher 표면 | `og_reach` 직접 | 차이 |
|---|---|---|---|
| 4홉 | 195,202 페이지 / 193.4 ms | 16,170 페이지 / 19.0 ms | **179,032 페이지** |
| 6홉 | 222,561 페이지 / 267.8 ms | 41,910 페이지 / 67.1 ms | 180,651 페이지 |

4홉에서 도달 노드는 약 50,000개이므로, 노드 하나당 약 3.6 페이지가 C4+C5에 쓰인다(추정 — 179,032 / 50,000).
이것이 이 코드베이스에서 가장 큰 단일 최적화 여지다
([`PERF-02`](07_improvements_performance.md), [`PERF-03`](07_improvements_performance.md)).

## 5. 사실 — 쓰기 한 건의 비용 구성

읽기와 달리 쓰기는 SQL 한 문장이 아니라 **Rust에서 행 단위로 SPI를 호출하는 절차**다
([`engine/src/cypher/mod.rs:236-243`](../../../engine/src/cypher/mod.rs) 의 per-env 루프).

| 항목 | 무엇을 하는가 | 비용 |
|---|---|---|
| W1 카탈로그 조회 | graph_id / type_id / kind / storage_table | SPI 4회, 캐시 없음 |
| W2 id 할당 | `og_id_alloc` `INSERT … ON CONFLICT DO UPDATE … RETURNING` | SPI 1회 + **타입당 행 1개에 대한 배타 락** |
| W3 프로퍼티 계획 | `og_catalog.property` 조회 + 미선언 프로퍼티 승격 판정 | SPI 2회 (+ 신규 프로퍼티마다 DDL) |
| W4 레지스트리 INSERT | `og_data.og_node` / `og_edge` | SPI 1회 |
| W5 타입 테이블 INSERT | `og_data.n_*` / `e_*` | SPI 1회 |
| W6 양방향 인접 갱신 | `og_adj` 의 `'o'`/`'i'` 세그먼트 각각 append | SPI 2~4회, **세그먼트 튜플 전체 재작성** |

정확한 횟수는 [`03_hot_paths.md` §3](03_hot_paths.md) 에서 코드를 세어 산출한다.
W6의 append는 `SET nbr = a.nbr || $4` 형태의 `UPDATE`
([`adjacency.rs:22-31`](../../../engine/src/storage/adjacency.rs))이므로,
이웃 1개를 추가할 때 MVCC 규칙상 **최대 4 KB짜리 튜플 전체가 새 버전으로 다시 쓰인다.**
이것이 이 설계가 읽기에서 얻은 이득의 대가다.

## 6. 결정 — 이 모델이 담고 있는 설계 결정

| # | 결정 | 대가 |
|---|---|---|
| D1 | 인접을 엣지 1행이 아니라 노드당 세그먼트(≤256 이웃)로 저장 | 쓰기 증폭. 이웃 1개 추가 = 세그먼트 튜플 재작성 |
| D2 | 프로퍼티를 실컬럼으로 (jsonb 블롭 아님) | 스키마 변경이 DDL이 됨. 미선언 프로퍼티는 쓰기 시점 승격 |
| D3 | 라벨을 구간(nested-set) 인덱스로 | 계층 변경 시 전체 재라벨링 + 전 뷰 무효화 |
| D4 | `access.sql` 을 전부 `LANGUAGE sql` 로 (인라인 가능) | **`WITH RECURSIVE` 를 가진 `og_vlp`/`og_reach_sql` 은 예외** — §7 참고 |
| D5 | Cypher를 함수 호출(`og_cypher`)로 진입 | 최상위 문장이 `PARALLEL UNSAFE` 함수 호출이 됨 |
| D6 | 결과를 jsonb 객체 1개/행으로 반환 | 프로젝션마다 jsonb 조립. `count(DISTINCT b)` 가 노드 전체를 비교 |

## 7. 사실 — "access.sql은 전부 인라인된다"는 주장의 정확한 범위

[`access.sql:1-9`](../../../engine/sql/access.sql) 의 주석은 모든 함수가 인라인되어
"플래너가 순회 스캔 자체를 본다"고 말한다. 코드를 읽고 확인한 정확한 범위는 다음과 같다.

| 함수 | 본문 | 인라인 가능성 | Cypher 핫패스에서 호출되는가 |
|---|---|---|---|
| `og_expand`, `og_expand_batch` | 단일 SELECT | 가능 | **아니오** — 컴파일러는 `og_adj … LATERAL unnest` 를 직접 생성한다 (`compile.rs:900-904`) |
| `og_subtype_ids` | 단일 SELECT | 가능 | 아니오 |
| `og_nodes`, `og_edges` | `IN (SELECT …)` 서브링크 포함 | 의심스러움 (미확인) | 아니오 — 엔진 내 호출자 없음 |
| `og_vlp` | `WITH RECURSIVE` | **불가로 추정** | 예 |
| `og_reach_sql` | `WITH RECURSIVE` | **불가로 추정** | 아니오 (컴파일러가 방출하지 않음) |
| `og_node_json`, `og_edge_json` | `LANGUAGE plpgsql` + 동적 `EXECUTE` | **불가 (확정)** | 예 — 타입이 컴파일 시점에 확정되지 않은 노드의 모든 프로퍼티 접근 (`compile.rs:991`, `:1013`) |

- `og_expand` 의 실제 호출자는 [`portal/server/index.js:262,266`](../../../portal/server/index.js) 와
  하네스의 `ontological_raw` 행([`bench/harness.py:398-401`](../../../bench/harness.py))뿐이다.
- `og_nodes` / `og_edges` / `og_subtype_ids` 는 `engine/src/` 어디에서도 호출되지 않는다
  (`grep` 으로 확인). 즉 **브리핑이 우려한 `type_id IN (SELECT og_subtype_ids(...))` 형태는
  Cypher 계획에 등장하지 않는다.** 라벨 스캔은 항상 타입 뷰의 `UNION ALL` 이다.
- `LANGUAGE sql` SRF의 인라인 여부는 PostgreSQL의 `inline_set_returning_function()` 이 결정하며,
  CTE를 포함한 본문은 인라인되지 않는 것으로 알려져 있다. **이 문서는 그 동작을 소스로 확인하지 않았다.**
  확인 방법: `EXPLAIN` 출력에 `Function Scan on og_vlp` 노드가 나타나면 인라인되지 않은 것이다.

## 8. 사실 vs 추정 — 이 문서의 신뢰도 구분

| 진술 | 종류 | 근거 |
|---|---|---|
| 한 홉이 위 §1의 SQL로 컴파일된다 | 사실(코드) | `compile.rs:900-906` |
| `og_adj` 한 행 = 이웃 ≤256개, 4 KB, TOAST 없음 | 사실(코드) | `bootstrap.sql:197-211` |
| 1홉 페이지: AGE 1,707 / 우리 1,742 / 스토리지 389 | 사실(측정) | `bench-50000-20260806T042833Z.json` |
| 4홉에서 Cypher 표면이 179,032 페이지를 더 읽는다 | 사실(측정) | `bench-50000-20260817T033001Z.json` |
| 그것이 도달 노드당 약 3.6 페이지다 | **추정** | 179,032 ÷ 약 50,000 (도달 노드 수는 4홉에서 그래프가 포화한다는 서술에 기댐) |
| `WHERE a.val = 7` 이 인덱스를 못 쓴다 | **추정** | `IS NOT DISTINCT FROM` 이 생성되는 것은 사실이나, 실제 계획은 확인하지 못함. `EXPLAIN` 필요 |
| `og_vlp` 가 인라인되지 않는다 | **추정** | PostgreSQL의 알려진 동작. `EXPLAIN` 으로 확인 필요 |
| 쓰기 경로의 실제 처리량 | **미확인** | `og_create_node`/`og_create_edge` 를 측정한 벤치가 없음 |
| 동시성 하 성능 | **미확인** | 모든 측정은 질의 1개 단독 실행 |
| 콜드 캐시 성능 | **미확인** | 모든 측정은 웜 캐시 |

<!-- affects: backend, data -->
<!-- requires-update: docs/01_architecture/09_performance/03_hot_paths.md, docs/01_architecture/09_performance/05_planner_interaction.md -->
