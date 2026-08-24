# 벡터 · 하이브리드 검색 API

> **이 문서가 답하는 질문**
> - 임베딩이 왜 별도 저장소가 아니라 프로퍼티인가?
> - 관계(엣지)에 임베딩을 다는 것이 왜 공짜인가?
> - `og_vector_search`의 `filter` 인자는 정확히 무엇을 받는가? (보안 주의)
> - 하이브리드 검색의 점수는 어떻게 계산되는가?
> - 임베딩 staleness는 어떻게 판정되는가?

---

## 1. 결정(Decision) — 임베딩은 프로퍼티다

별도의 임베딩 저장소가 없다. 임베딩은 `vector(N)` 프로퍼티이고, spec 002가 그것을
타입 테이블의 **실컬럼**으로 만들고, spec 001이 다른 컬럼과 똑같이 저장한다
([engine/src/vector/mod.rs:3](../../engine/src/vector/mod.rs#L3)).

이 결정 하나가 두 가지를 만든다.

1. **관계 임베딩이 1급이 된다** (spec 004 FR-002). 엣지 타입 테이블도 노드 타입
   테이블과 같은 방식으로 벡터 컬럼을 얻는다 — 같은 인덱스, 같은 트랜잭션, 같은 RLS.
2. **필터 푸시다운이 구조적이다** (FR-013). 벡터 검색이 돌 때 Cypher 컴파일러는
   이미 라벨을 구체 테이블로 해석해 놓았으므로, 그래프 술어와 ANN 인덱스가
   **같은 릴레이션 위에** 있다. 사후 필터가 숨을 곳이 없다.

**지원 메트릭** ([engine/src/vector/mod.rs:19](../../engine/src/vector/mod.rs#L19))

| `metric` 인자 | 연산자 | pgvector opclass |
|---|---|---|
| `cosine` | `<=>` | `vector_cosine_ops` |
| `l2` \| `euclidean` | `<->` | `vector_l2_ops` |
| `ip` \| `inner_product` \| `dot` | `<#>` | `vector_ip_ops` |

그 외는 `unknown metric '<m>' (cosine | l2 | ip)` ([vector/mod.rs:24](../../engine/src/vector/mod.rs#L24)).

**점수 정규화 (사실)**: 코사인과 내적은 거리를 유사도로 바꿔 **항상 클수록 좋게**
만든다([vector/mod.rs:119](../../engine/src/vector/mod.rs#L119)).

| 메트릭 | `score` 식 |
|---|---|
| `cosine` | `1 - (col <=> query)` |
| `ip` | `-(col <#> query)` |
| `l2` | `col <-> query` — **이 경우만 작을수록 가깝다** |

> ⚠️ `l2`는 정규화되지 않는다. `docs/api.md:139`의 "Scores are normalised so
> higher is always better"는 `cosine`/`ip`에만 해당한다
> → [12_improvements_api.md](12_improvements_api.md) **API-15**.

---

## 2. 임베딩 선언

### `og_add_embedding(graph text, type_name text, prop text, dims int4, metric text DEFAULT 'cosine', source_prop text DEFAULT NULL) RETURNS void`

정의: [engine/src/vector/mod.rs:32](../../engine/src/vector/mod.rs#L32) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값

**무엇을 하는가**: 노드 **또는 관계** 타입에 임베딩 슬롯을 선언하고 HNSW 인덱스를 계층 전체에 만든다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 엔티티 **또는 관계** 타입 이름 |
| `prop` | `text` | 필수 | — | 프로퍼티 이름 |
| `dims` | `int4` | 필수 | — | 차원 수. `1..16000` |
| `metric` | `text` | 선택 | `'cosine'` | `cosine` / `l2` / `ip` |
| `source_prop` | `text` | 선택 | `NULL` | 이 임베딩이 파생된 원본 프로퍼티. staleness 판정에 사용(FR-022) |

**반환**: 없음.

**부수 효과 (순서대로)**
1. `og_add_property(graph, type_name, prop, 'vector(<dims>)', false, false)` —
   **일반 프로퍼티 경로를 그대로 재사용한다. 이것이 요점이다**
   ([vector/mod.rs:48](../../engine/src/vector/mod.rs#L48)).
2. 서브타입 전체에 `CREATE INDEX IF NOT EXISTS hnsw_<sub>_<col> ON <table> USING hnsw (<col> <opclass>)`.
3. `og_catalog.embedding`에 upsert (`ON CONFLICT (type_id, prop) DO UPDATE`).

**예제** ([examples/demo.sql:51](../../examples/demo.sql#L51))

```sql
-- An embedding on a RELATIONSHIP type.
SELECT og_add_embedding('default', 'COLLABORATED_WITH', 'context', 4, 'cosine', 'note');
-- And on a node type.
SELECT og_add_embedding('default', 'Work', 'summary_vec', 4, 'cosine', 'tagline');
```

**실패 조건**

| 조건 | 오류 | 위치 |
|---|---|---|
| `dims ∉ 1..16000` | `embedding dimension <n> is out of range (1..16000)` | [vector/mod.rs:42](../../engine/src/vector/mod.rs#L42) |
| 알 수 없는 메트릭 | `unknown metric '<m>' (cosine \| l2 \| ip)` | [vector/mod.rs:24](../../engine/src/vector/mod.rs#L24) |
| 프로퍼티 이미 선언됨 | `failed to declare embedding property: <e>` | [vector/mod.rs:53](../../engine/src/vector/mod.rs#L53) |
| HNSW 인덱스 생성 실패 | `failed to build HNSW index: <e>` | [vector/mod.rs:62](../../engine/src/vector/mod.rs#L62) |

> ⚠️ **`dims` 상한 16000은 pgvector HNSW의 상한이 아니다.** pgvector의 HNSW
> 인덱스는 2000차원에서 멈춘다. `dims > 2000`이면 프로퍼티 선언은 성공하고
> **인덱스 생성 단계에서** `failed to build HNSW index: …`로 실패한다
> ([engine/src/compat/genai.rs:157](../../engine/src/compat/genai.rs#L157)의 주석이 이
> 한계를 명시한다) → [12_improvements_api.md](12_improvements_api.md) **API-16**.
> 4096차원 모델은 `genai.dimensions`로 잘라 쓸 것([09_neo4j_compat.md](09_neo4j_compat.md)).

---

## 3. 검색

### `og_vector_search(graph text, type_name text, prop text, query text, k int4 DEFAULT 10, filter text DEFAULT NULL) RETURNS TABLE(id int8, score float8, entity jsonb)`

정의: [engine/src/vector/mod.rs:94](../../engine/src/vector/mod.rs#L94) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 노드 **또는 관계** 타입에 대한 top-k 시맨틱 검색.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 타입 이름. 서브타입 포함 뷰가 스캔 대상 |
| `prop` | `text` | 필수 | — | `og_add_embedding`으로 선언한 프로퍼티 |
| `query` | `text` | 필수 | — | 질의 벡터의 **텍스트 표현**. pgvector 리터럴 `'[0.1,0.2,…]'` |
| `k` | `int4` | 선택 | `10` | 반환 행 수. **SQL에 리터럴로 삽입된다** |
| `filter` | `text` | 선택 | `NULL` | ⚠️ **SQL 불리언 조각.** 아래 경고 참조 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `id` | `int8` | 아니오 | 노드 또는 엣지 id |
| `score` | `float8` | 아니오 | §1의 점수 식 |
| `entity` | `jsonb` | 아니오 | `og_node_json(id)` 또는 `og_edge_json(id)`. 없으면 `{}` |

**차원 검사 (확인된 구현)**: `query` 문자열의 **쉼표 개수 + 1**을 차원으로 본다
([vector/mod.rs:134](../../engine/src/vector/mod.rs#L134)).

```
ERROR:  query vector has 3 dimension(s) but 'Work.summary_vec' is declared as vector(4)
```

> ⚠️ 이 검사는 문자열 파싱이 아니라 쉼표 세기다. 공백이나 형식이 어긋난
> 입력에는 무의미한 판정을 내릴 수 있다 → API-17.

> 🔒 **`filter`는 SQL 텍스트로 그대로 보간된다.**
> ```rust
> Some(f) if !f.trim().is_empty() => format!("AND ({f})")
> ```
> ([vector/mod.rs:116](../../engine/src/vector/mod.rs#L116))
> **금지**: 최종 사용자 입력을 `filter`로 넘기지 말 것. 임의 SQL 실행 경로다.
> 이는 "푸시다운을 구조적으로 만들기 위한" 의도된 설계지만
> ([vector/mod.rs:92](../../engine/src/vector/mod.rs#L92)), 계약상 신뢰 경계가
> 문서화되어 있지 않다 → [12_improvements_api.md](12_improvements_api.md) **API-05**.

**예제**

```sql
SELECT id, score, entity ->> 'title' AS title
  FROM og_vector_search('default', 'Work', 'summary_vec',
                        '[0.9,0.1,0.1,0.2]', 5);

-- filter is a SQL predicate on the type's own columns (trusted input only)
SELECT id, score
  FROM og_vector_search('default', 'Work', 'summary_vec',
                        '[0.9,0.1,0.1,0.2]', 5, 'p_year > 2000');
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 임베딩 미선언 | `no embedding named '<prop>' is declared on this type` ([vector/mod.rs:86](../../engine/src/vector/mod.rs#L86)) |
| 차원 불일치 | `query vector has <n> dimension(s) but '<T>.<p>' is declared as vector(<d>)` |
| `filter` SQL 오류 / 벡터 파싱 실패 | `vector search failed: <pg error>` ([vector/mod.rs:144](../../engine/src/vector/mod.rs#L144)) |

---

### `og_vector_search_exact(graph text, type_name text, prop text, query text, k int4 DEFAULT 10) RETURNS TABLE(id int8, score float8)`

정의: [engine/src/vector/mod.rs:411](../../engine/src/vector/mod.rs#L411) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 인덱스를 끄고 전수 탐색한다. ANN 재현율(recall) 측정용 정답지(spec 004 FR-028).

**인자**: `og_vector_search`와 같되 `filter`가 없다.

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `id` | `int8` | 아니오 | id |
| `score` | `float8` | 아니오 | ⚠️ **정규화되지 않은 원(raw) 거리** — `(col <op> query)` 그대로 ([vector/mod.rs:430](../../engine/src/vector/mod.rs#L430)) |

> ⚠️ **`og_vector_search`와 반환 형태·점수 의미가 모두 다르다.**
> `entity` 컬럼이 없고, 코사인이어도 `1 - distance` 변환을 하지 않아
> **작을수록 가깝다**. 두 함수의 `score`를 직접 비교하면 안 된다
> → [12_improvements_api.md](12_improvements_api.md) **API-15**.

**구현 사실**: `SET LOCAL enable_indexscan = off` → 질의 → `SET LOCAL enable_indexscan = on`
([vector/mod.rs:428](../../engine/src/vector/mod.rs#L428), [:440](../../engine/src/vector/mod.rs#L440)).
인덱스 스캔을 끄는 것이 요점이다 — 이것이 참조 구현이다.

**차원 검사는 하지 않는다** — `og_vector_search`에는 있는 검사가 여기엔 없다.
잘못된 차원은 `exact search failed: <pg error>`가 된다.

**예제**

```sql
WITH truth AS (SELECT id FROM og_vector_search_exact('default','Work','summary_vec','[0.9,0.1,0.1,0.2]',10)),
     ann   AS (SELECT id FROM og_vector_search      ('default','Work','summary_vec','[0.9,0.1,0.1,0.2]',10))
SELECT count(*)::float8 / 10 AS recall_at_10
  FROM truth JOIN ann USING (id);
```

---

### `og_similar(graph text, id int8, prop text, k int4 DEFAULT 10) RETURNS TABLE(id int8, score float8, entity jsonb)`

정의: [engine/src/vector/mod.rs:158](../../engine/src/vector/mod.rs#L158) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: "이것과 비슷한 것 찾기". **관계에도 동작한다**(FR-012).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | ⚠️ **무시된다** — `let _ = graph;` ([vector/mod.rs:173](../../engine/src/vector/mod.rs#L173)). 타입은 `id`의 비트에서 얻는다 |
| `id` | `int8` | 필수 | — | 기준이 되는 노드 또는 엣지 id |
| `prop` | `text` | 필수 | — | 임베딩 프로퍼티 이름 |
| `k` | `int4` | 선택 | `10` | 반환 행 수 |

**반환**: `og_vector_search`와 동일 (`id`, `score`, `entity`). 기준 요소 자신은 제외된다(`v.id <> $1`).

**스캔 범위 결정**: `root_type_of(tid)`로 **계층의 루트 타입**을 찾아 그 뷰를 스캔한다
([vector/mod.rs:170](../../engine/src/vector/mod.rs#L170), [:204](../../engine/src/vector/mod.rs#L204)).
즉 형제 서브타입까지 포함해 비교한다.

**예제** ([examples/demo.sql:136](../../examples/demo.sql#L136))

```sql
-- similarity between RELATIONSHIPS, not just nodes
SELECT id, score, entity ->> 'note'
  FROM og_similar('default', 549755813889, 'context', 3);
```

**실패 조건**
- 임베딩 미선언 → `no embedding named '<prop>' is declared on this type`
- 실행 실패 → `similarity search failed: <pg error>` ([vector/mod.rs:191](../../engine/src/vector/mod.rs#L191))

> ⚠️ `graph` 인자가 무시되므로, 다른 그래프의 id를 넘겨도 조용히 동작한다
> → [12_improvements_api.md](12_improvements_api.md) **API-18**.

---

### `og_hybrid_search(graph text, type_name text, prop text, query text, anchor int8 DEFAULT NULL, k int4 DEFAULT 10, vector_weight float8 DEFAULT 1.0, graph_weight float8 DEFAULT 1.0) RETURNS TABLE(id int8, score float8, vector_score float8, graph_score float8, entity jsonb)`

정의: [engine/src/vector/mod.rs:222](../../engine/src/vector/mod.rs#L222) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 벡터 유사도와 그래프 근접성을 **Reciprocal Rank Fusion(RRF)** 으로 융합한다(FR-018..021).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 타입 이름 |
| `prop` | `text` | 필수 | — | 임베딩 프로퍼티 |
| `query` | `text` | 필수 | — | 질의 벡터 텍스트 |
| `anchor` | `int8` | 선택 | `NULL` | 결과가 그래프상 가까워야 할 기준 노드. `NULL`이면 그래프 점수는 전부 0 |
| `k` | `int4` | 선택 | `10` | 반환 행 수 |
| `vector_weight` | `float8` | 선택 | `1.0` | 벡터 순위 항의 가중치 |
| `graph_weight` | `float8` | 선택 | `1.0` | 그래프 근접 항의 가중치 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `id` | `int8` | 아니오 | 노드 id |
| `score` | `float8` | 아니오 | 융합 점수 (`fscore`) |
| `vector_score` | `float8` | 아니오 | 정규화된 벡터 유사도 |
| `graph_score` | `float8` | 아니오 | `1 / (1 + hops)`. anchor 없거나 미도달이면 `0` |
| `entity` | `jsonb` | 아니오 | `og_node_json(id)` |

**점수 공식 (코드 그대로, [vector/mod.rs:273](../../engine/src/vector/mod.rs#L273))**

```
fscore = vector_weight * (1.0 / (60 + vrank))
       + graph_weight  * COALESCE(1.0 / (60 + hops), 0)
```

- `vrank`는 벡터 거리 순 `row_number()`.
- RRF 상수 `60`은 **하드코딩**이다.
- 후보 풀 크기는 `max(k * 10, 50)` ([vector/mod.rs:248](../../engine/src/vector/mod.rs#L248)).
- 그래프 근접성은 `og_vlp(anchor, NULL, 'b'::"char", 0, 3)` 의
  `min(depth)` — **깊이 3이 하드코딩**되어 있고, 도달성(`og_reach`)이 아니라
  **트레일 열거(`og_vlp`)** 를 쓴다([vector/mod.rs:253](../../engine/src/vector/mod.rs#L253)).

> ⚠️ **관계 타입에는 동작하지 않는다.** `ensure_view(tid, false)`로 항상 노드 뷰를
> 만들고 `og_node_json(id)`로 결과를 만든다([vector/mod.rs:247](../../engine/src/vector/mod.rs#L247),
> [:276](../../engine/src/vector/mod.rs#L276)). `og_vector_search`/`og_similar`는
> `is_edge`를 판별하는데 이 함수만 하지 않는다
> → [12_improvements_api.md](12_improvements_api.md) **API-19**.

> ⚠️ `k`, `vector_weight`, `graph_weight`, `anchor`가 모두 **SQL 텍스트로 포맷된다**.
> `NaN`/`Infinity` 같은 `float8` 값은 유효하지 않은 SQL을 만든다.

**예제**

```sql
SELECT id, score, vector_score, graph_score, entity ->> 'title'
  FROM og_hybrid_search('default', 'Work', 'summary_vec',
                        '[0.9,0.1,0.1,0.2]',
                        anchor => 412316860417,
                        k => 5,
                        vector_weight => 1.0,
                        graph_weight => 2.0);
```

**실패 조건**
- 임베딩 미선언 → `no embedding named '<prop>' is declared on this type`
- 실행 실패 → `hybrid search failed: <pg error>` ([vector/mod.rs:283](../../engine/src/vector/mod.rs#L283))

---

## 4. Staleness 관리

**결정(Decision)**: 임베딩이 원본 프로퍼티에서 파생되었음을 `source_prop`으로
기록하고, 원본 값의 **MD5 해시**를 `og_data.og_embedding_state`에 저장해 비교한다.

### `og_stale_embeddings(graph text) RETURNS TABLE(entity_id int8, type_name text, prop text)`

정의: [engine/src/vector/mod.rs:299](../../engine/src/vector/mod.rs#L299) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 원본 프로퍼티가 바뀌어 무효가 된 임베딩을 나열한다(FR-022/023).

**판정 조건** ([vector/mod.rs:339](../../engine/src/vector/mod.rs#L339)):
```sql
WHERE x.<source_col> IS NOT NULL
  AND (x.<embedding_col> IS NULL
       OR s.source_hash IS DISTINCT FROM md5(x.<source_col>::text))
```

즉 **`source_prop`이 선언된 임베딩만** 검사한다
(`WHERE ... e.source_prop IS NOT NULL`, [vector/mod.rs:310](../../engine/src/vector/mod.rs#L310)).

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `entity_id` | `int8` | 아니오 | 노드 또는 엣지 id |
| `type_name` | `text` | 아니오 | 선언된 타입 이름 |
| `prop` | `text` | 아니오 | 임베딩 프로퍼티 이름 |

**예제**

```sql
SELECT * FROM og_stale_embeddings('default');
--  entity_id   | type_name | prop
-- 412316860417 | Work      | summary_vec
```

**실패 조건**: 그래프 없음 → `graph '<g>' does not exist`.
개별 타입 스캔이 실패하면 **조용히 건너뛴다**(`.unwrap_or_default()`,
[vector/mod.rs:347](../../engine/src/vector/mod.rs#L347)) — 결과가 불완전할 수 있다.

### `og_mark_embedded(entity_id int8, prop text) RETURNS void`

정의: [engine/src/vector/mod.rs:358](../../engine/src/vector/mod.rs#L358) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 현재 원본 값 기준으로 임베딩이 최신임을 기록한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `entity_id` | `int8` | 필수 | — | 노드 또는 엣지 id |
| `prop` | `text` | 필수 | — | 임베딩 프로퍼티 이름 |

**반환**: 없음.

> ⚠️ **조용히 무연산이 되는 경우가 둘 있다** ([vector/mod.rs:368](../../engine/src/vector/mod.rs#L368)):
> - `source_prop`이 선언되지 않은 임베딩 → 그냥 `return`
> - 타입에 저장 테이블이 없음 → 그냥 `return`
>
> 호출자는 성공/무시를 구별할 수 없다 → [12_improvements_api.md](12_improvements_api.md) **API-20**.

**전형적 워크플로**

```sql
-- 1. find what needs re-embedding
SELECT * FROM og_stale_embeddings('default');
-- 2. compute the vector out of band, write it back
SELECT og_set_node_props(412316860417, '{"summary_vec": "[0.9,0.1,0.1,0.2]"}'::jsonb);
-- 3. record that it is current
SELECT og_mark_embedded(412316860417, 'summary_vec');
```

### `og_embedding_stats(graph text) RETURNS jsonb`

정의: [engine/src/vector/mod.rs:383](../../engine/src/vector/mod.rs#L383) · 휘발성: `STABLE` · 병렬: `STRICT`

**무엇을 하는가**: 선언된 임베딩 목록을 반환한다.

**반환 구조**

```json
{
  "graph": "default",
  "embeddings": [
    {"type": "COLLABORATED_WITH", "property": "context",
     "dims": 4, "metric": "cosine", "source_property": "note"},
    {"type": "Work", "property": "summary_vec",
     "dims": 4, "metric": "cosine", "source_property": "tagline"}
  ]
}
```

정렬은 `t.name, e.prop` ([vector/mod.rs:391](../../engine/src/vector/mod.rs#L391)).

---

## 5. Cypher / Neo4j 경로에서의 진입

같은 기능이 Cypher 표면에도 있다.

| Cypher | 도달 지점 |
|---|---|
| `vector.similarity(a, b)` | `1 - (a::vector <=> b::vector)` |
| `vector.distance(a, b)` | `a::vector <=> b::vector` |
| `vector.l2(a, b)` | `a::vector <-> b::vector` |
| `CALL db.index.vector.queryNodes(name, k, vec) YIELD node, score` | `og_vector_search` ([compat/procs.rs:164](../../engine/src/compat/procs.rs#L164)) |
| `CREATE VECTOR INDEX … OPTIONS {indexConfig: {`vector.dimensions`: N}}` | `og_add_embedding` ([compat/ddl.rs:212](../../engine/src/compat/ddl.rs#L212)) |
| `genai.vector.encode(text, provider, config)` | `og_genai_encode` |

상세는 [09_neo4j_compat.md](09_neo4j_compat.md).

---

## 6. 금지 / 필수

- 🔒 **금지**: `og_vector_search(filter)`에 사용자 입력을 넣지 말 것. 임의 SQL이다.
- **금지**: `og_hybrid_search`를 관계 타입에 쓰지 말 것 — 노드 뷰로만 동작한다.
- **금지**: `og_vector_search`의 `score`와 `og_vector_search_exact`의 `score`를
  같은 척도로 비교하지 말 것. 후자는 원 거리다.
- **금지**: `l2` 메트릭에서 `score`를 "클수록 좋다"로 읽지 말 것.
- **필수**: `dims > 2000`이면 HNSW 인덱스가 만들어지지 않는다. `genai.dimensions`로
  잘라 쓰거나 인덱스 없는 운용을 감수할 것.
- **필수**: 질의 벡터는 pgvector 리터럴 문자열(`'[0.1,0.2]'`)로 넘길 것.
  차원 검사는 쉼표 개수 기준이다.
- **필수**: staleness 추적을 원하면 `og_add_embedding(..., source_prop => '…')`으로
  원본을 반드시 선언할 것 — 없으면 `og_stale_embeddings`와 `og_mark_embedded`가
  모두 그 임베딩을 무시한다.

---

## 7. 관련 문서

- 프로퍼티 선언과 물리 컬럼 → [01_graph_ddl.md](01_graph_ddl.md)
- `genai.vector.encode` 설정 → [09_neo4j_compat.md](09_neo4j_compat.md)
- 원문 요약 → [docs/api.md:126](../../docs/api.md)

<!-- affects: api, backend, data -->
<!-- requires-update: 02_api/09_neo4j_compat.md, 02_api/12_improvements_api.md -->
