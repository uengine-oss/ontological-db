# 07. 벡터 데이터 모델 — 임베딩, HNSW, staleness

> **이 문서가 답하는 질문**
> - 임베딩은 어디에 저장되는가? 노드와 엣지가 정말 같은 방식인가?
> - HNSW 인덱스는 어떤 파라미터로 만들어지고, 그것을 바꿀 수 있는가?
> - stale 임베딩은 어떻게 판정되고 어디에 기록되는가?
> - 하이브리드(RRF) 검색은 어떤 데이터를 읽는가?

**정본**: [`engine/src/vector/mod.rs`](../../engine/src/vector/mod.rs) (442줄),
[`engine/sql/bootstrap.sql:262-285`](../../engine/sql/bootstrap.sql),
[`engine/src/compat/ddl.rs:211-230`](../../engine/src/compat/ddl.rs).

---

## 결정 — 별도의 벡터 저장소가 없다

> "There is no separate embedding store. An embedding is a `vector(N)` property,
> which spec 002 turns into a real column on the type table and spec 001 stores
> like any other column."
> (`engine/src/vector/mod.rs:3-5`)

`og_add_embedding()`은 **일반 프로퍼티 경로를 그대로 재사용한다**:

```rust
Spi::run_with_args(
    "SELECT og_add_property($1, $2, $3, $4, false, false)",
    &[graph.into(), type_name.into(), prop.into(), format!("vector({dims})").into()],
)
```
(`engine/src/vector/mod.rs:49-53`, 주석: "Reuse the ordinary property path: this is the whole point.")

**이 한 결정에서 나오는 것**:

| 성질 | 왜 공짜인가 |
|---|---|
| **관계(엣지) 임베딩이 1급 시민** | 엣지 타입 테이블도 똑같이 컬럼을 얻는다 (`engine/src/vector/mod.rs:5-7`) |
| 트랜잭션 / MVCC | 그냥 컬럼이다 |
| RLS | 그냥 컬럼이다 |
| **필터 푸시다운이 구조적** | Cypher 컴파일러가 라벨을 구체 테이블로 해석해 놓았으므로, 그래프 술어와 ANN 인덱스가 **같은 관계** 위에 있다. 후처리 필터가 숨을 자리가 없다 (`engine/src/vector/mod.rs:9-12`) |
| `pg_dump` | 런타임 생성 사용자 테이블이므로 자동으로 덤프된다 |

---

## 사실 — 저장 형태

### 컬럼

`vector(N)`은 `map_data_type()`이 인식하는 유일한 파라미터형 타입이다:
```rust
if let Some(rest) = other.strip_prefix("vector(") {
    if let Some(dims) = rest.strip_suffix(')') {
        if dims.chars().all(|c| c.is_ascii_digit()) && !dims.is_empty() {
            return format!("vector({dims})");
        }
    }
}
```
(`engine/src/catalog/types.rs:29-38`)

컬럼 이름은 일반 규칙을 따른다 — `embedding` → `p_embedding`
(`engine/src/catalog/types.rs:53-66`).

차원 범위 검사:
```rust
if !(1..=16000).contains(&dims) {
    error!("embedding dimension {dims} is out of range (1..16000)");
}
```
(`engine/src/vector/mod.rs:41-43`)

**저장 크기**: pgvector의 `vector(N)`은 varlena이며 `4 * N + 8` 바이트다.
`vector(1536)` = 6,152 바이트 → **TOAST 임계(약 2,000바이트)를 훨씬 넘는다.**
이 컬럼에는 `SET STORAGE`가 지정되지 않으므로 기본 `EXTENDED`이고,
**TOAST 테이블로 나간다.**

> **미측정**: 실제 TOAST 비율은 측정하지 않았다. 확인 방법:
> ```sql
> SELECT c.relname, pg_size_pretty(pg_relation_size(c.oid)) AS heap,
>        pg_size_pretty(pg_relation_size(c.reltoastrelid)) AS toast
>   FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
>  WHERE n.nspname = 'og_data' AND c.relname LIKE 'n\_%' AND c.reltoastrelid <> 0;
> ```

**함의**: 임베딩 컬럼을 가진 타입 테이블은 순수 그래프 스캔에서도
TOAST 접근 비용을 지불할 수 있다. `SELECT *`를 피하고 필요한 컬럼만 읽어야 한다.
생성 뷰(`v_<tid>`)는 **모든 프로퍼티 컬럼을 투영하므로** `p_embedding`도 항상 목록에 있다
(`engine/src/cypher/views.rs:110-116`). PostgreSQL은 실제로 참조되지 않는
컬럼의 TOAST를 펼치지 않으므로 대개 문제되지 않지만,
`og_node_json()`처럼 `to_jsonb(x)`로 행 전체를 만드는 경로는 **반드시 펼친다**
(`engine/sql/access.sql:220`).

### 메타데이터

```sql
CREATE TABLE og_catalog.embedding (
    emb_id      int4 PRIMARY KEY,
    type_id     int4 NOT NULL REFERENCES og_catalog.type(type_id) ON DELETE CASCADE,
    prop        text NOT NULL,
    dims        int4 NOT NULL,
    metric      text NOT NULL DEFAULT 'cosine',
    source_prop text,
    UNIQUE (type_id, prop)
);
```
(`engine/sql/bootstrap.sql:266-274`)

**벡터 값은 여기 없다.** 검색 API가 필요로 하는 메타데이터만 있다
(`engine/sql/bootstrap.sql:263-265`). 조회는 상위 타입 라인까지 본다:
```sql
SELECT e.dims, e.metric FROM og_catalog.embedding e
 WHERE e.type_id = ANY($1) AND e.prop = $2 LIMIT 1
```
(`engine/src/vector/mod.rs:79-81`, `$1 = og_supertypes(tid)`)

---

## 사실 — HNSW 인덱스는 파라미터 없이 만들어진다

```rust
let (_, opclass) = metric_op(metric);
for sub in labeling::og_subtypes(tid) {
    if let Some(table) = types::storage_table(sub) {
        Spi::run(&format!(
            "CREATE INDEX IF NOT EXISTS hnsw_{sub}_{col} ON {table} \
             USING hnsw ({col} {opclass})"
        ))
        .unwrap_or_else(|e| error!("failed to build HNSW index: {e}"));
    }
}
```
(`engine/src/vector/mod.rs:56-64`)

**`WITH (m = ..., ef_construction = ...)` 절이 없다.**
확인: `engine/src/` 전체에서 `hnsw` 문자열의 매치는 위 두 줄
(`engine/src/vector/mod.rs:59-60`)뿐이고, `ef_construction` / `ivfflat` 매치는 0건이다.

따라서 **pgvector 기본값이 항상 쓰인다**:

| 파라미터 | 값 | 설정 가능한가 |
|---|---|---|
| `m` | pgvector 기본값 | **불가** — 코드에 전달 경로 없음 |
| `ef_construction` | pgvector 기본값 | **불가** |
| `hnsw.ef_search` (질의 시점) | 세션 기본값 | 확장이 `SET`하지 않음. 사용자가 세션에서 직접 설정해야 함 |

Neo4j 호환 DDL 경로도 마찬가지다. `CREATE VECTOR INDEX ... OPTIONS {indexConfig: {...}}`에서
읽는 것은 `vector.dimensions`와 `vector.similarity_function` **두 개뿐**이고,
나머지 옵션은 `og_catalog.compat_index.options` jsonb에 기록만 되고 사용되지 않는다
(`engine/src/compat/ddl.rs:212-229`, 기록은 `engine/src/compat/ddl.rs:250`).

→ [`10_improvements_data.md`](10_improvements_data.md) `DATA-13`

### 지표(metric)와 연산자

```rust
fn metric_op(metric: &str) -> (&'static str, &'static str) {
    match metric.to_ascii_lowercase().as_str() {
        "cosine"                    => ("<=>", "vector_cosine_ops"),
        "l2" | "euclidean"          => ("<->", "vector_l2_ops"),
        "ip" | "inner_product" | "dot" => ("<#>", "vector_ip_ops"),
        other => error!("unknown metric '{other}' (cosine | l2 | ip)"),
    }
}
```
(`engine/src/vector/mod.rs:19-26`)

**지표는 인덱스의 opclass에 굳는다.** 나중에 `og_add_embedding`을 다른 `metric`으로
다시 부르면 `og_catalog.embedding` 행은 `ON CONFLICT DO UPDATE`로 바뀌지만
(`engine/src/vector/mod.rs:66-74`), 인덱스는 `CREATE INDEX IF NOT EXISTS`라
**바뀌지 않는다.** 카탈로그와 인덱스가 어긋난 채로 남아 검색이 인덱스를 못 탄다.
→ `DATA-15`

점수는 "높을수록 좋다"로 정규화된다:
```rust
let score = match op {
    "<=>" => format!("(1 - (v.{col} {op} $1::vector))"),   // cosine  → similarity
    "<#>" => format!("(-(v.{col} {op} $1::vector))"),       // ip      → 부호 반전
    _     => format!("(v.{col} {op} $1::vector)"),          // l2      → 거리 그대로
};
```
(`engine/src/vector/mod.rs:119-124`)
`l2`만 "낮을수록 좋다"로 남는다는 점에 주의.

---

## 사실 — 검색이 읽는 관계

```rust
let view = crate::cypher::views::ensure_view(tid, is_edge);
let sql = format!(
    "SELECT v.id, {score}::float8 AS score, {json_fn}(v.id) AS entity
       FROM {view} v
      WHERE v.{col} IS NOT NULL {where_sql}
      ORDER BY v.{col} {op} $1::vector
      LIMIT {k}"
);
```
(`engine/src/vector/mod.rs:112, 126-132`)

**검색은 물리 테이블이 아니라 서브타입 합집합 뷰(`v_<tid>` / `ve_<tid>`)를 읽는다.**
이것이 "서브타입까지 포함한 검색"을 공짜로 만든다.

**대가**: 뷰가 `UNION ALL`이므로 PostgreSQL은 각 분기에 대해 HNSW 인덱스를 따로
쓰고 결과를 합친 뒤 다시 정렬한다. `LIMIT k`가 각 분기에 푸시다운되므로
분기 수 × k 후보를 모아 상위 k를 고르는 형태가 되어야 한다.
> **미측정**: 서브타입이 많을 때 실제 계획이 그렇게 나오는지 확인하지 않았다.
> 확인: `EXPLAIN (ANALYZE, BUFFERS) SELECT ... FROM og_data.v_<tid> ...`

`filter` 인자는 **SQL 불리언 조각 그대로** `AND ({f})`로 붙는다
(`engine/src/vector/mod.rs:115-118`). 인덱스와 같은 관계 위에서 평가되므로
푸시다운이 성립하지만, **사용자 문자열이 SQL에 그대로 들어간다.**
이 인자는 신뢰된 호출자만 채워야 한다.

`og_vector_search_exact()`는 리콜 측정용 기준선이다.
`SET LOCAL enable_indexscan = off`로 인덱스를 끄고 완전 탐색한다
(`engine/src/vector/mod.rs:427-431, 440`).
> **주의**: `SET LOCAL enable_indexscan = on`으로 되돌리지만(`engine/src/vector/mod.rs:440`),
> 원래 값이 `on`이었다고 가정한다. 오류로 중간에 빠져나가면 되돌리지 못한다 —
> `LOCAL`이라 트랜잭션 끝에 복구되므로 실제 피해는 없다.

---

## 사실 — staleness 추적

### 무엇을 기록하는가

```sql
CREATE TABLE og_data.og_embedding_state (
    entity_id   int8 NOT NULL,
    prop        text NOT NULL,
    source_hash text NOT NULL,
    embedded_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (entity_id, prop)
);
```
(`engine/sql/bootstrap.sql:279-285`)

`source_hash`는 **원본 프로퍼티의 md5**다:
```sql
INSERT INTO og_data.og_embedding_state (entity_id, prop, source_hash, embedded_at)
SELECT $1, $2, md5(x.{scol}::text), now() FROM {table} x WHERE x.id = $1
ON CONFLICT (entity_id, prop) DO UPDATE
   SET source_hash = EXCLUDED.source_hash, embedded_at = now()
```
(`engine/src/vector/mod.rs:372-377`)

### 어떻게 판정하는가

```sql
SELECT x.id FROM {table} x
  LEFT JOIN og_data.og_embedding_state s
    ON s.entity_id = x.id AND s.prop = '{prop}'
  WHERE x.{scol} IS NOT NULL
    AND (x.{ecol} IS NULL
         OR s.source_hash IS DISTINCT FROM md5(x.{scol}::text))
```
(`engine/src/vector/mod.rs:336-341`)

**stale의 정의**: 원본이 있는데 ① 임베딩이 없거나 ② 기록된 해시가 지금 원본의
해시와 다르다.

**중요한 성질**
- **자동 기록이 없다.** 임베딩을 계산한 뒤 `og_mark_embedded(entity_id, prop)`을
  **호출해야만** 상태가 기록된다(`engine/src/vector/mod.rs:358-381`).
  부르지 않으면 그 엔티티는 영원히 stale이다.
- **`source_prop`이 NULL이면 추적하지 않는다** (`engine/src/vector/mod.rs:310`,
  `AND e.source_prop IS NOT NULL`). `og_add_embedding`의 `source_prop`은 기본 NULL이므로,
  **명시하지 않으면 staleness 추적이 아예 꺼진다.**
- **트리거가 없다.** 원본 프로퍼티를 갱신해도 `og_embedding_state`는 그대로다.
  `og_stale_embeddings()`를 호출하는 시점에 md5를 다시 계산해 비교한다.
- **`og_stale_embeddings()`는 전체 스캔이다.** `{table}` × 서브타입마다
  `og_embedding_state`와 LEFT JOIN하고 `md5()`를 행마다 계산한다.
  인덱스가 쓰일 수 있는 조건이 없다. 큰 그래프에서는 배치 작업으로만 쓸 것.
- `{prop}`가 SQL 문자열에 보간된다(`engine/src/vector/mod.rs:338`).
  프로퍼티 이름은 카탈로그에서 온 값이라 사용자 임의 입력은 아니지만,
  작은따옴표를 이스케이프하지 않는다.

### 삭제 시 정리

`og_embedding_state`는 **어떤 삭제 경로에서도 정리되지 않는다.**
- `delete_node_inner`(`engine/src/storage/mod.rs:355-383`) — 언급 없음
- `og_drop_type`(`engine/src/catalog/types.rs:700-709`) — 언급 없음
- `og_drop_graph`(`engine/src/catalog/types.rs:321-340`) — 언급 없음

→ 고아 행이 무한히 쌓인다. → `DATA-07`

---

## 사실 — 하이브리드 검색(RRF)이 읽는 것

```sql
WITH prox AS (SELECT node, min(depth) AS hops
                FROM og_vlp({anchor}::int8, NULL, 'b'::"char", 0, 3) GROUP BY node),
     cand AS (SELECT v.id, {score}::float8 AS vscore,
                     row_number() OVER (ORDER BY v.{col} {op} $1::vector) AS vrank
                FROM {view} v WHERE v.{col} IS NOT NULL
               ORDER BY v.{col} {op} $1::vector LIMIT {pool}),
     fused AS (SELECT c.id, c.vscore,
                      COALESCE(1.0 / (1.0 + p.hops), 0)::float8 AS gscore,
                      {vector_weight} * (1.0 / (60 + c.vrank))
                    + {graph_weight}  * COALESCE(1.0 / (60 + p.hops), 0) AS fscore
                 FROM cand c LEFT JOIN prox p ON p.node = c.id)
SELECT id, fscore::float8, vscore::float8, gscore::float8, og_node_json(id)
  FROM fused ORDER BY fscore DESC LIMIT {k}
```
(`engine/src/vector/mod.rs:251-278`)

**데이터 모델 관점의 관찰**

| 관찰 | 근거 |
|---|---|
| 후보 풀은 `max(k*10, 50)` | `engine/src/vector/mod.rs:248` |
| RRF 상수는 60으로 **하드코딩** | `engine/src/vector/mod.rs:273-274` |
| 그래프 근접성은 **`og_vlp`**를 쓴다 — `og_reach`가 아니다 | `engine/src/vector/mod.rs:253` |
| 홉 상한이 **3으로 하드코딩** | 같은 줄 |
| `og_node_json(id)`를 결과 k행에 대해 호출 | `engine/src/vector/mod.rs:276` |
| 엣지 임베딩은 지원하지 않음 (`ensure_view(tid, false)`) | `engine/src/vector/mod.rs:247` |

`og_vlp`를 쓰는 것이 문제다. `og_vlp`는 **트레일(경로) 열거**이므로
행 수가 `degree^k`로 자란다(`engine/src/storage/traverse.rs:3-10`).
평균 차수 20인 그래프에서 3홉이면 앵커 하나당 8,000행이고, `GROUP BY node`가
그것을 압축하기 **전에** 다 만들어진다. `og_reach`는 방문 집합을 쓰므로
같은 답을 `|V| + |E|` 안에서 낸다. → `PERF-14`

---

## 금지 / 필수

**금지**
- 임베딩 프로퍼티를 `og_add_property(..., 'jsonb')`나 `'float8[]'`로 선언하는 것.
  반드시 `og_add_embedding()`을 쓸 것 — 그래야 `vector(N)`이 되고 HNSW를 걸 수 있다.
- 임베딩 컬럼을 가진 타입에 `og_enable_history()`를 켜는 것.
  이력 트리거가 `to_jsonb(NEW)`로 **행 전체**를 담으므로, 매 갱신마다 벡터 전체가
  `og_history.payload`에 직렬화된다(`engine/sql/access.sql:285`). → `PERF-12`
- `og_vector_search(..., filter)`에 신뢰되지 않은 문자열을 넘기는 것.
  SQL 조각 그대로 붙는다.
- 지표를 바꾸려고 `og_add_embedding`을 다시 부르는 것. 인덱스는 안 바뀐다.
  기존 `hnsw_*` 인덱스를 먼저 `DROP INDEX` 할 것.

**필수**
- `og_add_embedding(..., source_prop => '<원본 프로퍼티>')`를 **반드시 지정**할 것.
  안 하면 staleness 추적이 꺼진다.
- 임베딩을 계산한 뒤 `og_mark_embedded(id, prop)`을 호출할 것.
- 대량 임베딩 적재 시에는 HNSW 인덱스를 먼저 드롭하고, 적재 후 재생성할 것.
  (확장은 이를 자동으로 하지 않는다.)
- 리콜을 조정하려면 세션에서 직접:
  ```sql
  SET hnsw.ef_search = 100;   -- 기본값보다 높이면 리콜↑ 지연↑
  ```
  그리고 `og_vector_search_exact()`로 기준선을 재서 비교할 것.
- 인덱스 존재 확인:
  ```sql
  SELECT i.indexrelid::regclass AS index, i.indrelid::regclass AS table,
         am.amname
    FROM pg_index i
    JOIN pg_class c  ON c.oid = i.indexrelid
    JOIN pg_am    am ON am.oid = c.relam
   WHERE am.amname = 'hnsw';
  ```

---

<!-- affects: data, backend, search -->
<!-- requires-update: docs/06_data/08_data_lifecycle.md, docs/06_data/10_improvements_data.md -->
