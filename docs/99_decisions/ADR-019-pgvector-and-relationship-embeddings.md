# ADR-019: 벡터 저장·인덱싱은 pgvector를 재사용하고, 임베딩을 관계에도 부여한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/004-vector-hybrid-search/plan.md` 기준일) |
| 영향 범위 | vector, storage, catalog, cypher |
| 근거 | `.specify/memory/constitution.md` 원칙 V, `engine/src/vector/mod.rs:1-13`, `:55-65`, `specs/004-vector-hybrid-search/plan.md` 설계 결정 1·2, Complexity Tracking |

> **이 문서가 답하는 질문**
> - 왜 ANN 인덱스를 직접 구현하지 않았는가?
> - "관계에도 임베딩"이 왜 별도의 기능이 아니라 공짜인가?
> - 사후 필터링이 왜 불가능한가?

## 배경

헌법 원칙 V는 세 가지를 동시에 요구한다.

1. 임베딩이 **노드뿐 아니라 관계(엣지)와 경로/서브그래프**에도 붙어야 한다.
2. 벡터 저장·인덱싱은 **자체 구현하지 않고 pgvector를 재사용**한다 (원칙 I의 귀결).
3. 하이브리드 검색에서 **사후 필터링(post-filter)은 금지** — 그래프 술어가 벡터 탐색
   내부로 push-down 되어야 한다.

## 고려한 선택지

1. **자체 ANN 인덱스 구현** — 원칙 I·V 위반. 헌법이 pgvector를 *"벡터 저장/인덱싱의 유일한
   구현체"* 로 지정했다.
2. **별도 임베딩 저장소(벡터 전용 테이블/외부 벡터 DB)** — 원칙 VI("이중 저장 구조 금지")
   위반. 트랜잭션·백업·RLS가 갈라진다.
3. **임베딩을 그냥 프로퍼티로 두고, pgvector가 그 컬럼에 인덱스를 건다**

## 결정

**3안.** 임베딩은 `vector(N)` 타입의 **프로퍼티**다. ADR-005가 그것을 타입 테이블의
실제 컬럼으로 만들고, `og_add_embedding`이 그 컬럼에 HNSW 인덱스를 건다
(`engine/src/vector/mod.rs:55-65`).

```sql
CREATE INDEX IF NOT EXISTS hnsw_<sub>_<col> ON <table> USING hnsw (<col> <opclass>)
```

`og_add_embedding`은 **일반 프로퍼티 경로를 그대로 재사용**한다
(`engine/src/vector/mod.rs:48`: *"Reuse the ordinary property path: this is the whole
point."*).

## 근거

- 관계 임베딩이 1급인 이유가 별도 기능이 아니라 **ADR-005의 귀결**임이 명시되어 있다
  (`engine/src/vector/mod.rs:3-8`):
  > There is no separate embedding store. An embedding is a `vector(N)` property, which
  > spec 002 turns into a real column on the type table and spec 001 stores like any
  > other column. That single decision is what makes **relationship embeddings first
  > class** (FR-002): an edge type table gets a vector column the same way a node type
  > table does, with the same index, transactions and RLS.
- 사후 필터링이 **구조적으로 불가능**한 이유 (`engine/src/vector/mod.rs:9-13`):
  > by the time a vector search runs, the Cypher compiler has already resolved the label
  > to concrete tables, so the graph predicate and the ANN index live on the *same
  > relation*. There is nowhere for a post-filter to hide.
- 경로 선택(강한 필터면 인덱스 스캔, 약하면 HNSW)은 우리가 규칙을 만들지 않고 **플래너에
  위임**한다 (`specs/004-.../plan.md` 설계 결정 3).
- 재현율에 대한 이탈이 정직하게 기록되어 있다 (`plan.md` Complexity Tracking):
  > 재현율 보장을 pgvector `hnsw.ef_search` 튜닝에 의존 | ANN 재현율은 인덱스 파라미터의
  > 함수다. 자체 구현은 원칙 I·V 위반 | 사후 필터링은 스펙이 금지. 정확 탐색 강제는 성능
  > 목표 미달

## 결과

**긍정적**
- 관계 임베딩이 노드 임베딩과 **완전히 같은 코드 경로**를 탄다. README가 이를 제품 차별점의
  하나로 든다: *"pgvector on nodes **and relationships**"*.
- MVCC·트랜잭션·RLS·`pg_dump`가 임베딩에도 그대로 적용된다.
- 그래프 술어와 ANN 인덱스가 같은 릴레이션에 있으므로 push-down이 구조적이다.

**부정적 / 감수한 대가**
- **재현율이 pgvector의 `hnsw.ef_search` 튜닝에 종속된다.** 우리가 보장할 수 있는 것은
  측정(재현율 하네스)이지 알고리즘이 아니다.
- 차원이 1~16000으로 제한된다 (`engine/src/vector/mod.rs:41-43`) — pgvector의 한계다.
- `og_add_embedding`이 하위 타입 테이블마다 인덱스를 만든다 (`:57-65`). 타입 계층이 넓으면
  인덱스 수가 그만큼 늘어난다.
- 임베딩이 배열 값이므로 ADR-006의 자동 승격 대상이 아니다. 반드시 `og_add_embedding`으로
  선언해야 하며, 그 보호 장치가 `WIDENABLE` 가드다 (ADR-006).
- 원칙 V가 요구한 **경로/서브그래프 임베딩**은 노드·관계만큼 명확히 구현되어 있지 않다.
  이 ADR은 노드·관계에 대해서만 근거를 갖는다.

## 재검토 조건

- pgvector가 요구 성능·재현율을 충족하지 못하는 사례가 측정으로 확인되면 — 그때도
  자체 구현이 아니라 **pgvector 업스트림 기여 또는 다른 표준 확장**을 먼저 검토한다
  (원칙 I).
- 16000차원 상한이나 HNSW 파라미터 제약이 실제 워크로드를 막을 때.
- 경로/서브그래프 임베딩을 정식 지원하기로 하면, "임베딩 = 프로퍼티" 모델이 경로에도
  성립하는지(경로는 저장된 개체가 아니다) 별도 ADR이 필요하다.

<!-- affects: vector, storage, catalog, cypher -->
<!-- requires-update: docs/99_decisions/ADR-005-typed-property-columns.md -->
