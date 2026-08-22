# 포지셔닝 — Neo4j / Apache AGE / TypeDB / pgGraph 대비

> **이 문서가 답하는 질문**
> - 이미 Neo4j / AGE / TypeDB / pgGraph 가 있는데 왜 이것을 쓰는가?
> - 각 대안 대비 **무엇을 얻고 무엇을 포기하는가**?
> - 어떤 상황에서는 이것을 **쓰지 말아야** 하는가?

> **인용 규칙**: 이 문서의 모든 수치는 [`docs/benchmark.md`](../benchmark.md) 와
> [`docs/deep-traversal.md`](../deep-traversal.md) 에서 온 실측값이다.
> 두 문서는 하네스가 **답이 서로 다르면 타이밍을 무효화**하도록 되어 있다
> ([`bench/harness.py`](../../bench/harness.py), [`README.md`](../../README.md)).

---

## 한 장 요약

| | Apache AGE | pgGraph | Neo4j 5 | TypeDB 3 | **Ontological** |
|---|---|---|---|---|---|
| 어디서 도는가 | PostgreSQL 확장 | PostgreSQL 확장 | 전용 서버(JVM) | 전용 서버 | **PostgreSQL 확장** |
| 종류 | 그래프 **저장소** | 그래프 **인덱스** | 그래프 저장소 | 타입 시스템 + 저장소 | 그래프 **저장소** |
| 진실의 원천 | AGE 라벨 테이블 | 기존 관계형 테이블 | 자체 저장소 | 자체 저장소 | `og_data.*` |
| 질의 언어 | openCypher | 고정 알고리즘 함수 + GQL 1.0 프로파일 | Cypher | TypeQL | **Cypher + TypeQL** |
| 프로퍼티 | `agtype` (JSON) | 원본 테이블 좌표 | 자체 포맷 | 속성 인스턴스 | **실 타입 컬럼** |
| 타입 상속 | 없음 | 없음 | 멀티라벨(수동) | 있음 | **구간 라벨 상수 시간** |
| role 제약 | 없음 | 없음 | 없음 | 있음 | 있음 |
| 벡터 | 없음 | 없음 | 노드 인덱스 | 없음 | **노드 + 엣지** |
| MVCC/RLS 유지 | 유지 | **포기** | 해당 없음 | 해당 없음 | **유지** |
| 백업 | `pg_dump` | 재빌드 | 별도 | 별도 | **`pg_dump`** |

근거: [`docs/comparison.md:19-36`](../comparison.md) 표를 확장한 것.

---

## Facts — 측정된 숫자

### 1/2/3홉 (50,000 노드 / 974,936 엣지, 중앙값)

| 질의 | Ontological (Cypher) | Ontological (storage) | Neo4j 5 | Apache AGE | AGE (`*1..n` 없이) | TypeDB 3 | 재귀 CTE |
|---|---|---|---|---|---|---|---|
| 1홉 | 2.10 ms | 0.31 ms | 0.74 ms | 2.15 ms | 2.15 ms | 0.46 ms | **0.24 ms** |
| 2홉 | 3.96 ms | 0.50 ms | 0.92 ms | 799.50 ms | 7.99 ms | 1.48 ms | **0.36 ms** |
| 3홉 | 33.86 ms | 4.33 ms | **2.99 ms** | 22,412 ms | 34.63 ms | 19.73 ms | 3.49 ms |
| 프로퍼티 스캔 | 0.62 ms | 0.24 ms | 0.68 ms | 0.27 ms | 0.24 ms | 0.52 ms | **0.19 ms** |

출처: [`README.md`](../../README.md) → [`docs/benchmark.md`](../benchmark.md).
재현: `python3 bench/harness.py --scale 50000 --degree 20`

### 깊은 순회 (같은 데이터셋, "시작 노드를 제외한 k홉 이내 distinct 노드 수")

| 깊이 | Ontological (Cypher) | Ontological (`og_reach`) | 재귀 CTE | Apache AGE | pgGraph | Neo4j 5 |
|---|---|---|---|---|---|---|
| 1 | 1.52 | 0.11 | **0.08** | 94.00 | 1.16 | 0.96 |
| 2 | 2.64 | 0.18 | **0.19** | 761.43 | 10.00 | 2.94 |
| 3 | 29.06 | **1.06** | 2.17 | 13,696.72 | 237.87 | 4.66 |
| 4 | 193.35 | **18.96** | 27.53 | *>60 s* | 2,123.56 | 63.27 |
| 5 | 251.53 | **54.45** | 170.37 | — | 2,533.34 | 131.33 |
| 6 | 267.75 | **67.10** | 374.39 | — | 2,457.50 | 168.82 |
| 8 | 270.31 | **70.51** | 788.02 | — | 2,540.89 | 151.67 |

출처: [`docs/deep-traversal.md`](../deep-traversal.md) "50,000 nodes / 974,936 edges" 표.

---

## 정직하게 말해야 하는 네 가지

이 프로젝트의 README가 직접 밝히고 있는 내용이며, 여기서도 그대로 옮긴다.

1. **3홉에서 Neo4j가 11배 빠르다.** 그리고 스케일에 따른 지연 증가도 Neo4j가 완만하다
   (26배 데이터에 3.7배 vs 우리 9.4배). 이건 넘어야 할 숫자이지, 이미 넘은 숫자가 아니다.
2. **손으로 쓴 재귀 CTE가 거의 모든 구간에서 더 빠르다.** 질의 엔진도, 타입 시스템도,
   Cypher도 없는 대신 빠르다. 이걸 감추면 나머지 숫자의 가치가 떨어진다.
3. **AGE의 붕괴는 저장 구조가 아니라 `*1..n` 연산자다.** 같은 질문을 고정 길이 패턴의
   합집합으로 물으면 AGE는 22,412 ms가 아니라 34.63 ms에 답한다. 그 열에 대해 우리는
   660배 앞선 게 아니라 **동등**하다. 예전 README에 있던 "AGE 대비 615배"는 엣지 엔드포인트에
   인덱스가 없는 AGE와 비교한 수치였고, 게시되지 말았어야 했다.
4. **Cypher 표면이 raw storage 경로 대비 3홉에서 7.8배 비싸다** (33.86 ms vs 4.33 ms).
   대부분 jsonb 프로젝션과 SPI 왕복 비용이다. 이것이 질의 표면의 현재 가격이다.

출처: [`README.md`](../../README.md) "Four things in that table deserve to be said out loud".

---

## 대안별 상세

### vs Apache AGE

**AGE가 하는 것**: PostgreSQL 위에 openCypher를 얹는다. 라벨마다 테이블을 만들고
(`g."Person" INHERITS (g._ag_label_vertex)`), 프로퍼티는 `agtype` 컬럼 하나에 담고,
Cypher는 `cypher('g', $$…$$)` 함수의 문자열 인자로 받는다.

**차이**

| | Apache AGE | Ontological |
|---|---|---|
| 인접 | 엣지당 1행 + B-tree | CSR형 세그먼트, 힙 튜플당 이웃 ≤256 |
| 프로퍼티 | `agtype` JSON 블롭 | 타입 카탈로그가 생성한 실 타입 컬럼 |
| 식별자 | `graphid` | `int8`, `[shard:9][type:18][local:36]` |
| Cypher | `cypher()` 의 문자열 인자 | 파싱 → 플래너가 최적화하는 평범한 SQL로 컴파일 |
| 상속 | 멀티라벨 수동 관리 | 구간 인덱스 계층, 상수 시간 서브타입 판정 |
| 벡터 | — | pgvector, 노드 **와 관계** 모두 |

**정정**: "AGE는 플래너가 아무것도 못 본다"는 서술은 **정확하지 않다**.
AGE도 `cypher()` 호출을 질의 트리로 재작성한다. 다만 `agtype` 때문에 컬럼 통계와
일반 인덱스가 없어서 조인 순서를 데이터가 아니라 기본값으로 고른다
([`docs/comparison.md`](../comparison.md) "A correction to how this repository has described AGE").

**AGE를 골라야 할 때**: openCypher 커버리지가 더 중요하고 (`UNION`, `shortestPath` 등),
깊은 순회를 쓰지 않으며, 이미 AGE 위에 데이터가 있을 때.

---

### vs pgGraph

**pgGraph가 하는 것**: 그래프를 **저장하지 않는다**. 이미 있는 관계형 테이블의 위상만
읽어서 인메모리 CSR 배열로 컴파일한다. `graph.build()` 가 `.pggraph` 아티팩트를 쓰고,
각 백엔드가 그것을 백엔드 로컬 익명 메모리에 매핑한다.

**얻는 것**: 포인터 없는 뜨거운 루프. CSR 워크가 깊이 20까지 **42 ms에서 평평**하다
([`docs/deep-traversal.md`](../deep-traversal.md) "pgGraph: the traversal is fast, the row is expensive").

**포기하는 것**: 패턴 질의, 타입 시스템, 트랜잭션 가시성, RLS, 스키마 진화.
2,458 ms라는 6홉 수치 중 **42 ms만이 CSR 워크**이고 나머지는 도달한 노드마다
11컬럼 행 하나를 만드는 비용이다.

**Ontological이 같은 내기를 한 곳**: `og_csr_build()` / `og_csr_reach()` 가 정확히 같은
구조다 — 백엔드 로컬 CSR, `u32` 밀집 인덱스, SPI 없음, 힙 없음, 플래너 없음.
6홉 4.9 ms. **차이는 자동으로 라우팅하지 않는다는 점**이다:
스냅샷이 빌드 시점에 얼어붙고 RLS를 전혀 참조하지 않기 때문이다
([`engine/src/storage/traverse.rs:19-25, 355-359`](../../engine/src/storage/traverse.rs)).
백엔드마다 8.4~9.2 MiB / 119~229 ms의 빌드 비용을 새 연결마다 다시 낸다
([`docs/deep-traversal.md`](../deep-traversal.md) "Per backend, not per database").

**pgGraph를 골라야 할 때**: 이미 정규화된 관계형 스키마가 진실의 원천이고,
그래프는 "가끔 도달성만 묻는" 부가 질문이며, RLS/트랜잭션 가시성이 필요 없을 때.

---

### vs Neo4j

**Neo4j가 잘하는 것**: 3홉 2.99 ms, 스케일에 따른 완만한 증가.
성숙한 드라이버 생태계, GDS 알고리즘 라이브러리, 브라우저.

**Ontological이 주는 것**
- **드라이버를 안 바꿔도 된다**: Bolt 4.4 게이트웨이가 있고, Neo4j 공식 MCP 서버
  (`mcp-neo4j-cypher`)가 무수정으로 동작한다 ([`examples/meeting-rooms/`](../../examples/meeting-rooms/)).
- **타입 상속과 role 제약**: Neo4j의 멀티라벨이 표현하지 못하는 것.
  `ACTED_IN` 의 source가 `Person` 이 아니면 데이터베이스가 거부한다
  ([`engine/src/storage/mod.rs:454-484`](../../engine/src/storage/mod.rs)).
- **관계 임베딩**: Neo4j 벡터 인덱스는 노드 대상이다. 여기서는 엣지 타입 테이블도
  똑같이 `vector(N)` 컬럼과 HNSW를 갖는다.
- **RLS가 순회 중간에 적용된다**: 컴파일된 질의가 평범한 테이블을 읽으므로,
  볼 수 없는 노드는 조인되지 않고 그 노드를 지나는 모든 경로가 사라진다
  ([`engine/src/interop/mod.rs:1-8`](../../engine/src/interop/mod.rs)).
- **하나의 트랜잭션**: 주문 테이블 UPDATE와 그래프 CREATE가 같은 트랜잭션이다.

**Neo4j를 골라야 할 때**: 3홉 이내 지연시간이 SLA이고, GDS 알고리즘이 필요하고,
운영 스택을 하나 더 두는 비용을 감당할 수 있을 때.

**Bolt 게이트웨이의 한계** ([`bolt/README.md`](../../bolt/README.md) 지원 매트릭스):
Bolt 3.x/5.x 미지원, `Path` 구조체 미인코딩(경로 변수는 홉 리스트로 도착),
시공간 타입 미지원(조용히 뭉개지 않고 거부), TLS 미종단(앞단 프록시 전제),
`neo4j://` 라우팅은 단일 서버 응답만.

---

### vs TypeDB

**TypeDB가 잘하는 것**: entity/relation/attribute와 role의 개념 모델, 추론.
이 프로젝트의 헌법 원칙 IV가 명시적으로 TypeDB를 벤치마킹 대상으로 지목한다.

**Ontological이 주는 것**
- **TypeDB의 예제가 그대로 돈다**: TypeDB 공식 bookstore 예제의 `schema.tql` / `data.tql` 이
  바이트 단위로 무수정 적재되고, 예제 README의 질의가 문서에 적힌 그대로의 결과를 낸다
  ([`README.md`](../../README.md) "Running a TypeDB example",
  [`examples/typedb/bookstore/`](../../examples/typedb/bookstore/),
  `python3 tests/typeql/run.py`).
- **같은 그래프를 Cypher로도 묻는다**: TypeQL이 관계를 노드로 reify하고 속성을
  값 중복 제거된 공유 인스턴스로 저장하는 매핑이 숨겨져 있지 않다 —
  `og_typeql_attribute` / `og_typeql_role` 뷰로 SQL에서 직접 읽힌다
  ([`engine/sql/access.sql:297-338`](../../engine/sql/access.sql)).
- **PostgreSQL 생태계 전부**.

**정직한 한계**: TypeDB **함수(`fun`)** 는 파싱·저장·덤프까지만 되고 평가되지 않는다.
bookstore README의 4개 질의 중 2개가 함수를 쓰므로, **4개 중 2개가 오늘 동작한다.**
호출하면 추측하지 않고 명시적으로 오류를 낸다
([`engine/src/typeql/compile.rs:486-488`](../../engine/src/typeql/compile.rs)).

**성능**: TypeDB 3은 3홉 19.73 ms로 우리 Cypher 표면(33.86 ms)보다 빠르고
raw storage 경로(4.33 ms)보다 느리다.

---

## Decisions — 포지셔닝 판단

1. **"AGE보다 N배 빠르다"를 마케팅 문구로 쓰지 않는다.**
   조건에 따라 동등하거나 뒤진다. 하네스가 정답 일치를 먼저 확인하고, 불일치 시 타이밍을
   무효화한다. (헌법 원칙 X)
2. **pgGraph의 내기를 옵트인으로만 제공한다.**
   `og_csr_build()` 는 존재하지만 자동 라우팅되지 않는다. MVCC/RLS를 조용히 잃는 경로를
   기본값으로 두지 않는다는 판단이다.
3. **경쟁이 아니라 대체 비용으로 포지셔닝한다.**
   "Neo4j보다 빠르다"가 아니라 "Neo4j를 두 번째 운영 스택으로 두지 않아도 된다"가 논지다.

---

## 이것을 쓰지 말아야 할 때 (Facts)

- 3홉 이내 지연시간이 SLA인 워크로드 — Neo4j가 11배 빠르다.
- 순수 도달성만 필요하고 타입/패턴/RLS가 필요 없을 때 — 재귀 CTE 또는 pgGraph가 더 낫다.
- `UNION`, `shortestPath`, `CALL {}`, GDS가 필요할 때 — 미지원이다
  ([`docs/cypher.md`](../cypher.md), [`bolt/README.md`](../../bolt/README.md)).
- 쓰기 확장을 위해 샤딩이 필요할 때 — 설계만 있고 구현이 없다 (spec 007).
- SPARQL 엔드포인트가 필요할 때 — 미구현이다 (spec 006 partial).

---

## Forbidden / Required

**Forbidden**
- "615배 빠르다" 류의 폐기된 수치를 인용하지 말 것. README가 그 수치를 명시적으로 철회했다.
- 벤치마크 표에서 유리한 열만 골라 인용하지 말 것. 재귀 CTE 열과 Neo4j 3홉 열을 같이 보일 것.
- pgGraph를 "느리다"고 요약하지 말 것. CSR 워크는 42 ms로 평평하고, 나머지는 행 출력 비용이다.

**Required**
- 성능 주장에는 데이터셋 규모, 홉 수, 측정 문서 링크를 항상 함께 붙일 것.
- 대안 대비 서술에는 "포기하는 것"을 반드시 함께 적을 것.

<!-- affects: overview, architecture, operations -->
<!-- requires-update: 00_overview/05_spec_status.md, 01_architecture/04_storage_architecture.md -->
