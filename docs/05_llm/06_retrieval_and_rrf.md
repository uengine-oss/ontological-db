# 06. 검색 계층 — 벡터 검색과 하이브리드 RRF

> **이 문서가 답하는 질문**
> - `og_vector_search` 가 실제로 생성하는 SQL은 무엇인가? 필터 푸시다운은 어떻게 성립하는가?
> - `og_hybrid_search` 의 RRF 공식은 정확히 무엇이고, 상수 `60` 은 어디에 있는가?
> - 기본 가중치에서 그래프 근접성과 벡터 순위 중 무엇이 순위를 지배하는가?
> - 관계(엣지) 임베딩이 검색에 주는 것은 무엇이고, 하이브리드에서는 왜 못 쓰는가?
> - RAG 검색 계층으로 볼 때 **없는 것**은 무엇인가?

---

## 1. 사실 — `og_vector_search` 가 만드는 SQL

정의: [engine/src/vector/mod.rs:94-155](../../engine/src/vector/mod.rs).

```rust
// engine/src/vector/mod.rs:119-132
let score = match op {
    "<=>" => format!("(1 - (v.{col} {op} $1::vector))"),   // cosine → 유사도
    "<#>" => format!("(-(v.{col} {op} $1::vector))"),      // ip → 부호 반전
    _     => format!("(v.{col} {op} $1::vector)"),         // l2 → 거리 그대로
};
let sql = format!(
    "SELECT v.id, {score}::float8 AS score, {json_fn}(v.id) AS entity
       FROM {view} v
      WHERE v.{col} IS NOT NULL {where_sql}
      ORDER BY v.{col} {op} $1::vector
      LIMIT {k}"
);
```

실제 생성 예 (graph=`kb`, type=`Doc`, prop=`emb`, metric=`cosine`, k=10):

```sql
SELECT v.id, (1 - (v.p_emb <=> $1::vector))::float8 AS score, og_node_json(v.id) AS entity
  FROM og_data.v_5 v
 WHERE v.p_emb IS NOT NULL
 ORDER BY v.p_emb <=> $1::vector
 LIMIT 10
```

### 1.1 점수 정규화 — "높을수록 좋다"

| metric | 저장 연산자 | 반환 `score` | 범위 |
|---|---|---|---|
| `cosine` | `<=>` | `1 - distance` | 정규화 벡터에서 대략 `[-1, 1]` |
| `ip` | `<#>` | `-distance` (pgvector의 `<#>` 는 음의 내적) | 내적 값 |
| `l2` | `<->` | **거리 그대로** | `[0, ∞)` — 낮을수록 좋다 |

**주의**: `l2` 에서는 `score` 가 거리이므로 "높을수록 좋다"가 깨진다. 코드 주석은
"Cosine/IP distances are converted to a similarity so higher is always better"
([vector/mod.rs:119](../../engine/src/vector/mod.rs))라고 하는데, l2는 변환 대상이 아니다.
`ORDER BY` 는 항상 거리 오름차순이므로 결과 **순서는 올바르다** —
잘못될 수 있는 것은 애플리케이션이 `score` 에 임계값을 걸 때다.

### 1.2 필터 푸시다운이 성립하는 이유

`{view}` 는 타입과 그 서브타입의 저장 테이블을 `UNION ALL` 로 묶은 뷰다
([engine/src/cypher/views.rs:93-138](../../engine/src/cypher/views.rs)). 라벨 해소가
**컴파일 시점**에 구체 테이블 목록으로 끝나 있으므로
([views.rs:14-17](../../engine/src/cypher/views.rs) 주석), 필터 술어와 HNSW 인덱스가
같은 릴레이션 위에 놓인다. 모듈 주석의 표현
([vector/mod.rs:9-12](../../engine/src/vector/mod.rs)):

> There is nowhere for a post-filter to hide.

**단, ANN 인덱스 내부에서의 필터 처리는 pgvector의 몫이다.** 저장소에는
`hnsw.ef_search` 도 `hnsw.iterative_scan` 도 설정하는 코드가 없다(전수 grep 0건).
spec 004 FR-015("낮은 선택도 필터에서 top-k 결과 개수가 부족해지는 상황을 방지해야
하며, 필요 시 탐색 범위를 자동 확대")
([specs/004-vector-hybrid-search/spec.md:203-204](../../specs/004-vector-hybrid-search/spec.md))
를 구현하는 코드는 없다. plan의 Complexity Tracking도 이를 인정한다
([specs/004-vector-hybrid-search/plan.md:86](../../specs/004-vector-hybrid-search/plan.md)):
"재현율 보장을 pgvector `hnsw.ef_search` 튜닝에 의존".

### 1.3 `filter` 인자는 원시 SQL 조각이다 — 보안 경계

```rust
// engine/src/vector/mod.rs:115-118
let where_sql = match filter {
    Some(f) if !f.trim().is_empty() => format!("AND ({f})"),
    _ => String::new(),
};
```

문자열이 그대로 SQL에 이어 붙는다. 이스케이프도 파싱도 없다.

spec 003 FR-026은 "사용자 값은 절대 SQL 텍스트로 보간하지 않는다"이고 Cypher 경로는
이를 지킨다([engine/src/cypher/compile.rs:1156-1157](../../engine/src/cypher/compile.rs) —
`($1 ->> 'name')`). **`og_vector_search` 의 `filter` 는 그 규칙의 명시적 예외**다.

- Cypher/Bolt 경로에서는 노출되지 않는다: `db.index.vector.queryNodes` 는
  `og_vector_search` 를 **인자 5개로만** 호출하고 filter를 넘기지 않는다
  ([engine/src/compat/procs.rs:188-194](../../engine/src/compat/procs.rs)).
- 위험은 **평문 SQL로 `og_vector_search` 를 직접 호출하는 경로**다.
  LLM이 생성한 문자열을 `filter` 로 넘기면 임의 SQL 실행이다.

`og_hybrid_search` 에는 `filter` 인자 자체가 없다
([vector/mod.rs:222-231](../../engine/src/vector/mod.rs)) — 즉 하이브리드 경로에는
메타데이터 필터링 수단이 없다.

### 1.4 `og_similar` 와 `og_vector_search_exact`

- `og_similar(graph, id, prop, k)` ([vector/mod.rs:158-202](../../engine/src/vector/mod.rs)):
  기준 엔티티를 CTE로 뽑아 같은 뷰에서 비교하고 자기 자신을 제외한다
  (`v.id <> $1`, [vector/mod.rs:184](../../engine/src/vector/mod.rs)).
  **`graph` 인자는 사용되지 않는다** ([vector/mod.rs:173](../../engine/src/vector/mod.rs) —
  `let _ = graph;`). 타입은 id에서 추출하고
  ([vector/mod.rs:165](../../engine/src/vector/mod.rs)), 검색 범위는 **루트 타입 전체**로
  넓혀진다 ([vector/mod.rs:170, 204-215](../../engine/src/vector/mod.rs) `root_type_of`).
  즉 `EV` 인스턴스로 `og_similar` 를 호출하면 `Vehicle` 계층 전체가 후보다.
- `og_vector_search_exact(graph, type, prop, query, k)`
  ([vector/mod.rs:411-442](../../engine/src/vector/mod.rs)): `SET LOCAL enable_indexscan = off`
  로 전수 탐색을 강제하는 재현율 측정용 기준선
  ([vector/mod.rs:428, 440](../../engine/src/vector/mod.rs)).
  **주의**: 반환 `score` 는 정규화되지 않은 **원시 거리**다
  ([vector/mod.rs:430](../../engine/src/vector/mod.rs)) — `og_vector_search` 의 `score` 와
  직접 비교할 수 없다. 비교는 id 집합으로만 해야 한다
  ([engine/tests/sql/03_vector_agent_rdf.sql:34-36](../../engine/tests/sql/03_vector_agent_rdf.sql) 가
  그렇게 한다).

---

## 2. 사실 — `og_hybrid_search` 의 RRF 공식

정의: [engine/src/vector/mod.rs:222-296](../../engine/src/vector/mod.rs).

시그니처:
```sql
og_hybrid_search(graph, type_name, prop, query,
                 anchor        := NULL,   -- int8, 그래프 근접의 기준 노드
                 k             := 10,
                 vector_weight := 1.0,
                 graph_weight  := 1.0)
→ TABLE(id, score, vector_score, graph_score, entity)
```

### 2.1 생성되는 SQL 전문

`anchor = 42`, `k = 10`, `metric = cosine`, `col = p_emb`, `view = og_data.v_5` 기준
(`pool = max(k*10, 50) = 100`, [vector/mod.rs:248](../../engine/src/vector/mod.rs)):

```sql
WITH prox AS (
       SELECT node, min(depth) AS hops
         FROM og_vlp(42::int8, NULL, 'b'::"char", 0, 3)
        GROUP BY node),
 cand AS (
       SELECT v.id,
              (1 - (v.p_emb <=> $1::vector))::float8 AS vscore,
              row_number() OVER (ORDER BY v.p_emb <=> $1::vector) AS vrank
         FROM og_data.v_5 v WHERE v.p_emb IS NOT NULL
        ORDER BY v.p_emb <=> $1::vector LIMIT 100),
 fused AS (
       SELECT c.id, c.vscore,
              COALESCE(1.0 / (1.0 + p.hops), 0)::float8 AS gscore,
              1 * (1.0 / (60 + c.vrank))
            + 1 * COALESCE(1.0 / (60 + p.hops), 0)      AS fscore
         FROM cand c LEFT JOIN prox p ON p.node = c.id)
 SELECT id, fscore::float8, vscore::float8, gscore::float8, og_node_json(id)
   FROM fused ORDER BY fscore DESC LIMIT 10
```

### 2.2 공식 (코드 그대로)

```
fscore = vector_weight × 1/(60 + vrank)
       + graph_weight  × COALESCE(1/(60 + hops), 0)
```

- `60` 은 **RRF의 k 상수**이며 [vector/mod.rs:273-274](../../engine/src/vector/mod.rs) 의
  `format!` 문자열 안에 **리터럴로 두 번 하드코딩**되어 있다. GUC도, `og_catalog.setting`
  키도, 함수 인자도 없다.
- `vector_weight` / `graph_weight` 는 **인자로 조정 가능**하며 f64 값이 SQL에
  직접 보간된다.
- `pool` = `(k * 10).max(50)` — 융합 후보 개수. 조정 불가
  ([vector/mod.rs:248](../../engine/src/vector/mod.rs)).
- 그래프 근접의 최대 홉은 **3** 으로 하드코딩, 방향은 `'b'`(양방향), 관계 타입 필터는
  `NULL`(전체) ([vector/mod.rs:253](../../engine/src/vector/mod.rs)).

### 2.3 이것은 RRF의 표준형이 아니다

표준 RRF는 **두 개의 순위(rank)** 를 융합한다. 여기서는 한쪽만 순위다.

| 신호 | 융합에 들어가는 값 | 성격 |
|---|---|---|
| 벡터 | `vrank` = `row_number()` (1..pool) | **순위** |
| 그래프 | `hops` = 앵커로부터 최소 홉 수 (0..3) | **거리** (순위 아님) |

`hops` 를 순위 자리에 넣으면 값 범위가 0..3으로 좁아진다. 기본 가중치 `(1.0, 1.0)`
에서의 실제 기여도:

| 항목 | 값 | 계산 |
|---|---|---|
| 벡터 기여 최대 (vrank=1) | **0.016393** | 1/61 |
| 벡터 기여 최소 (vrank=100) | **0.006250** | 1/160 |
| 그래프 기여 (hops=0) | **0.016667** | 1/60 |
| 그래프 기여 (hops=3) | **0.015873** | 1/63 |
| 그래프 기여 (도달 불가) | **0** | COALESCE |

따라서:

```
최악의 연결 노드 (vrank=100, hops=3) = 0.006250 + 0.015873 = 0.022123
최선의 비연결 노드 (vrank=1, hops=∞) = 0.016393 + 0        = 0.016393
```

**0.022123 > 0.016393.** 즉 **기본 가중치에서 `anchor` 를 지정하면, 앵커로부터 3홉
이내의 모든 노드가 3홉 밖의 모든 노드보다 무조건 앞선다.** 벡터 순위는 각 집단
*내부*의 순서만 결정한다. `pool` 을 키워도(k=50 → pool=500, vrank=500 → 0.001786)
결론은 같다.

이것이 결함인지 의도인지는 코드로 판정할 수 없다(주석에 언급 없음). 다만 spec 004
FR-018이 요구하는 것은 "벡터 유사도와 그래프 신호를 **결합**한 점수"
([specs/004-vector-hybrid-search/spec.md:211-212](../../specs/004-vector-hybrid-search/spec.md))
이고, 현재 동작은 결합이 아니라 **경성 분할(hard partition) + 조 내부 정렬**에 가깝다.

**회피 방법** (코드 변경 없이): `graph_weight` 를 벡터 기여도 스케일에 맞춰 낮춘다.
예를 들어 `graph_weight := 0.05` 면 그래프 기여가 0.00079~0.00083 이 되어 벡터 순위
차이(0.0063~0.0164)가 지배한다. `graph_weight := 0` 이면 순수 벡터 검색과 동일한
순서가 된다(FR-021 충족 — `fscore` 가 `vrank` 에 대해 단조 감소).

### 2.4 반환되는 구성 점수는 융합에 쓰인 값이 아니다

```rust
// engine/src/vector/mod.rs:271-274
SELECT c.id, c.vscore,                                   -- 코사인 유사도
       COALESCE(1.0 / (1.0 + p.hops), 0)::float8 AS gscore,  -- 1/(1+hops)
       {vw} * (1.0 / (60 + c.vrank))                     -- 실제 융합 항 (다른 식)
     + {gw} * COALESCE(1.0 / (60 + p.hops), 0) AS fscore -- 실제 융합 항 (다른 식)
```

| 반환 컬럼 | 반환되는 값 | 융합에 실제로 쓰인 값 |
|---|---|---|
| `vector_score` | `1 - cosine_distance` | `1/(60 + vrank)` |
| `graph_score` | `1/(1 + hops)` → {1, 0.5, 0.333, 0.25} | `1/(60 + hops)` → {0.0167 … 0.0159} |

spec 004 FR-020("결합 점수의 각 구성 요소는 개별적으로 확인 가능해야 한다",
[spec.md:214](../../specs/004-vector-hybrid-search/spec.md))의 취지와 어긋난다.
`vector_score + graph_score` 를 아무리 조합해도 `score` 를 재현할 수 없다.

### 2.5 하이브리드는 노드 전용이다

```rust
// engine/src/vector/mod.rs:247, 276
let view = crate::cypher::views::ensure_view(tid, false);   // is_edge = false 고정
… "SELECT id, fscore::float8, …, og_node_json(id) FROM fused …"
```

`is_edge` 가 `false` 로 고정되어 있고 JSON 변환도 `og_node_json` 고정이다.
`og_vector_search` / `og_similar` 는 `types::type_kind(tid) == 'r'` 로 분기해
엣지도 지원하지만([vector/mod.rs:111-113, 169-172](../../engine/src/vector/mod.rs)),
**`og_hybrid_search` 에 관계 타입을 넘기면 노드 뷰를 찾으므로 의도한 결과가 나오지
않는다.** spec 004의 차별 기능(FR-002, 관계 임베딩 1급)이 하이브리드 경로에는
적용되지 않는다.

### 2.6 `og_vlp` 를 쓰는 비용

`prox` CTE는 `og_vlp` 를 호출한다
([engine/sql/access.sql:138-156](../../engine/sql/access.sql)):

```sql
WITH RECURSIVE walk(node, depth, path) AS (
    SELECT src, 0, ARRAY[]::int8[]
  UNION ALL
    SELECT u.nbr, w.depth + 1, w.path || u.eid
      FROM walk w JOIN og_data.og_adj a ON a.src = w.node …
      CROSS JOIN LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
     WHERE w.depth < maxhop AND NOT (u.eid = ANY (w.path))
)
```

이것은 **경로(trail) 열거**다 — `path` 배열을 들고 다니며 중복 엣지를 배제한다.
`docs/deep-traversal.md` 가 설명하는 "경로 1개당 1행 열거"를 방문집합 BFS로 바꾼
재작성(`og_reach` / `og_csr_reach`)은 **이 경로에 적용되지 않았다.**
3홉 상한과 `GROUP BY node` 가 결과 크기는 줄이지만, 슈퍼노드가 앵커이거나 앵커 주변
차수가 크면 중간 결과가 폭발한다.

---

## 3. 사실 — RAG 검색 계층으로서 없는 것

| RAG 구성요소 | 상태 | 근거 |
|---|---|---|
| 벡터 검색 | ✅ | [vector/mod.rs:94-155](../../engine/src/vector/mod.rs) |
| 키워드/FTS 검색 | ✅ 단, **Cypher 표면 전용** — SQL 함수가 없다 | [engine/src/compat/procs.rs:203-240](../../engine/src/compat/procs.rs) |
| 벡터 + FTS **순위 결합** | ❌ **미구현** (FR-019 / T014) | [specs/004-vector-hybrid-search/tasks.md:29](../../specs/004-vector-hybrid-search/tasks.md) |
| 벡터 + 그래프 근접 결합 | ⚠️ 구현됨. 단 2.3/2.4절의 문제 | [vector/mod.rs:263-278](../../engine/src/vector/mod.rs) |
| 리랭킹(cross-encoder, LLM judge 등) | ❌ **없음** | 저장소 전수 검색에 리랭킹 코드 없음 |
| 청킹 | ❌ **없음** — 아래 3.1절 |
| MMR / 다양성 제어 | ❌ 없음 |
| ANN 재현율 튜닝 (`ef_search`) | ❌ 없음 | 1.2절 |
| ANN 재현율 **측정 하네스** | ❌ 없음. 3행 회귀 테스트 1건이 전부 | [engine/tests/sql/03_vector_agent_rdf.sql:34-36](../../engine/tests/sql/03_vector_agent_rdf.sql), `bench/harness.py` 에 recall 코드 부재 |
| 경로/서브그래프 임베딩 (FR-003) | ❌ 미구현 (T015) | [specs/004-vector-hybrid-search/tasks.md:30](../../specs/004-vector-hybrid-search/tasks.md) |
| groundedness / 근거성 판정 훅 | ❌ 없음 | [07_grounding_and_provenance.md](07_grounding_and_provenance.md) |

### 3.1 청킹 — 그래프 노드가 곧 청크다

임베딩은 타입 테이블의 `vector(N)` 컬럼이다(05 문서 1절). 따라서 **검색 단위 = 엔티티
1개**이고, 문서를 자르는 계층은 존재하지 않는다.

결과:

- 긴 텍스트를 가진 노드는 임베딩 1개로 압축된다. 임베딩 모델의 컨텍스트를 넘는 본문은
  모델 쪽에서 잘린다(DB는 관여하지 않음).
- 청크 단위 검색을 하려면 **청크를 노드 타입으로 모델링**해야 한다
  (예: `Doc` ─`HAS_CHUNK`→ `Chunk`). 이 경우 청크→문서 롤업은 애플리케이션의 몫이다.
- 같은 소스 프로퍼티에서 여러 청크를 만드는 것을 `og_add_embedding` 의 `source_prop`
  모델이 표현하지 못한다 — 슬롯 1개당 소스 프로퍼티 1개, 엔티티 1개당 벡터 1개
  ([engine/sql/bootstrap.sql:266-285](../../engine/sql/bootstrap.sql)).

**이것 자체는 나쁜 설계가 아니다.** 그래프에서는 청크가 이미 1급 엔티티일 수 있고,
그때는 청킹 계층이 중복이다. 다만 "문서를 넣으면 알아서 잘라준다"를 기대하면 안 된다.

---

## 4. 결정(Decision)

| ID | 결정 | 근거 |
|---|---|---|
| D-1 | pgvector에 위임. 자체 ANN 구현 없음 | [specs/004-vector-hybrid-search/plan.md:33](../../specs/004-vector-hybrid-search/plan.md) |
| D-2 | 선택도 기반 경로 전환을 PostgreSQL 플래너에 맡김 | [plan.md:18-19](../../specs/004-vector-hybrid-search/plan.md) |
| D-3 | 융합 기본값은 RRF (k=60), 가중합은 `vector_weight`/`graph_weight` 로 표현 | [vector/mod.rs:217-221, 273-274](../../engine/src/vector/mod.rs) |
| D-4 | 하이브리드의 "그래프 신호"는 앵커로부터의 홉 수 하나뿐 | [vector/mod.rs:251-256](../../engine/src/vector/mod.rs). FR-018이 열거한 관계 가중치·경로 수·중심성은 미구현 |
| D-5 | `filter` 는 원시 SQL 조각 — 성능(푸시다운)을 위해 안전성을 포기한 지점 | [vector/mod.rs:115-118](../../engine/src/vector/mod.rs) |

---

## 5. 필수(Required) / 금지(Forbidden)

**필수**

- `og_hybrid_search` 를 `anchor` 와 함께 쓸 때는 `graph_weight` 를 **명시적으로 튜닝**할 것.
  기본값 `1.0` 은 그래프 도달성을 경성 분할로 만든다 (2.3절).
- `score` 임계값을 쓸 거라면 metric이 `cosine` 인지 확인할 것. `l2` 슬롯의 `score` 는
  거리다 (1.1절).
- 재현율이 중요하면 세션에서 `SET hnsw.ef_search = …` 를 직접 걸 것. 코드는 걸지 않는다 (1.2절).
- `og_vector_search_exact` 와 비교할 때는 **id 집합**으로 비교할 것. `score` 스케일이 다르다 (1.4절).
- 관계 임베딩 검색에는 `og_vector_search` / `og_similar` 를 쓸 것 (2.5절).

**금지**

- LLM이 생성한 문자열을 `og_vector_search` 의 `filter` 인자로 전달하는 것 **절대 금지**.
  임의 SQL 실행 경로다 (1.3절).
- `og_hybrid_search` 의 `vector_score` / `graph_score` 를 `score` 의 구성 요소로 해석 금지 (2.4절).
- `og_hybrid_search` 에 관계(relation) 타입을 넘기지 말 것 (2.5절).
- 차수가 큰 노드를 `anchor` 로 지정하고 상한 없이 호출하지 말 것. `og_vlp` 는 경로 열거다 (2.6절).
- 이 DB가 "하이브리드 검색"을 한다는 것을 **벡터 + 전문검색(FTS) 융합**으로 오해하지 말 것.
  그 조합은 미구현이다 (3절).

---

## 6. 참고

- 원문: [docs/api.md:126-140](../../docs/api.md) "Vectors — spec 004"
- 스펙: FR-008~FR-021, SC-001~SC-006
  ([specs/004-vector-hybrid-search/spec.md:187-215, 250-263](../../specs/004-vector-hybrid-search/spec.md))
- 임베딩 수명주기: [05_embedding_pipeline.md](05_embedding_pipeline.md)
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-01, LLM-10, LLM-13, LLM-14, LLM-18

<!-- affects: llm, data, backend, security -->
<!-- requires-update: 02_api/00_index.md, 05_llm/05_embedding_pipeline.md -->
