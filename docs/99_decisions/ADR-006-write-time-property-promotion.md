# ADR-006: 미선언 프로퍼티를 쓰기 시점에 실컬럼으로 승격하고, 충돌 시 `text`로 단방향 확장한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-17 (커밋 `515e4e2`; 소스 주석에 2026-08-16 사고 이력) |
| 영향 범위 | storage, catalog, cypher, vector |
| 근거 | `engine/src/storage/mod.rs:76-160`, 특히 `:83-86`, `:64`, `:121-126` |

> **이 문서가 답하는 질문**
> - Neo4j 앱은 스키마를 선언하지 않는데, 그 프로퍼티들은 어떻게 실컬럼이 되는가?
> - 같은 프로퍼티에 서로 다른 타입이 들어오면 어떻게 되는가?
> - 왜 모든 컬럼을 넓히지 않고 일부만 넓히는가?

## 배경

ADR-005는 선언된 프로퍼티만 실컬럼이 되게 했다. 그런데 **Cypher 애플리케이션은 아무것도
선언하지 않는다** — Neo4j에는 선언할 스키마가 없기 때문이다
(`engine/src/storage/mod.rs:78-82`):
> Without this every property a Cypher app writes lands in the jsonb payload, where it
> cannot be indexed, has no statistics, and (before this) lost its type on the way out.

즉 ADR-005의 이점이 Cypher 경로에서는 거의 전부 증발한다. 이 문제는 공개된 Neo4j MCP
서버를 무수정으로 구동하는 작업(커밋 `515e4e2`) 중에 실전으로 드러났다.

## 고려한 선택지

1. **아무것도 하지 않음** — 미선언 프로퍼티는 계속 `__ext`에 남는다. Cypher 앱에게
   ADR-005는 사실상 무효.
2. **사용자에게 선언을 요구** — `og_add_property`를 먼저 호출하게 한다. Neo4j 드라이버를
   URI만 바꿔 붙인다는 호환성 목표(ADR-017/018)와 정면 충돌.
3. **쓰기 시점 자동 승격 + 타입 충돌 시 거부** — 승격은 하되 타입이 어긋나면 에러.
   Neo4j는 같은 프로퍼티가 노드마다 다른 타입을 갖는 것을 허용하므로, 정상 앱이 깨진다.
4. **쓰기 시점 자동 승격 + 충돌 시 `text`로 단방향 확장**

## 결정

**4안.** `declare_new_props`(`engine/src/storage/mod.rs:87`)가 쓰기 경로에서 다음을 한다.

- 값에서 컬럼 타입을 추론한다: `bool` / `int8` / `float8` / `text`
  (`infer_column_type`, `:47-61`). **스칼라만** 추론하며 배열·객체는 `__ext`에 남긴다.
- 처음 보는 이름이면 `og_add_property`를 호출해 실컬럼으로 만든다 (`:109-120`).
- 기존 컬럼과 타입이 어긋나면 **`text`로 확장**한다. 확장은 단방향이라 진동하지 않는다
  (`:83-86`).
- **확장 대상은 우리가 추론으로 만들 수 있었던 타입뿐이다** —
  `const WIDENABLE: &[&str] = &["bool", "int8", "float8"]` (`:64`).

## 근거

- 단방향 `text` 확장의 근거 (`engine/src/storage/mod.rs:83-86`):
  > Neo4j allows a property to hold different types on different nodes; text is the
  > one column type that can represent all of them, and widening is one-way so it
  > cannot oscillate.
- `WIDENABLE` 가드가 존재하는 이유는 **실제 사고 기록**이다 (`:121-126`):
  > Widen only what we could have inferred ourselves. A property declared as
  > `vector(1536)` or `timestamptz` was declared on purpose … and turning it into text
  > because one write disagreed would destroy that intent.
  > **(2026-08-16: doing so broke the vector suite, which is what this guard is for.)**
  즉 초기 구현은 모든 컬럼을 넓혔고, 그 결과 `og_add_embedding`이 만든 `vector(N)` 컬럼이
  `text`가 되어 벡터 스위트가 깨졌다. 가드는 그 회귀의 산물이다.
- 승격 시 하위 타입 테이블 전체를 순회하며, 별칭 뷰를 내렸다가 다시 세운다
  (`:130-145`) — PostgreSQL이 뷰가 의존하는 컬럼의 타입 변경을 거부하기 때문이다.

## 결과

**긍정적**
- Neo4j 앱이 선언 없이도 인덱싱 가능·통계 있는 실컬럼을 얻는다.
- 프로퍼티 타입이 왕복에서 보존된다.
- 의도적으로 선언된 타입(`vector(N)`, `timestamptz`, `numeric`)은 보호된다.

**부정적 / 감수한 대가**
- **쓰기 경로가 DDL을 실행할 수 있다.** 첫 쓰기가 `ALTER TABLE`을 유발하고 카탈로그 락을
  잡는다. 대량 동시 삽입 시 락 경합의 원인이 될 수 있다.
- `text` 확장은 **되돌릴 수 없다.** 실수로 문자열 하나를 쓰면 그 컬럼은 영구히 `text`이며,
  숫자 비교·인덱스 선택도가 함께 나빠진다.
- 확장이 하위 타입 전 테이블에 걸쳐 `ALTER TABLE ... USING`을 돌리므로 테이블 재작성이
  발생한다. 큰 타입에서는 저렴하지 않다.
- 배열/객체 값은 여전히 `__ext`에 남는다 — 이는 `embedding`을 지키기 위한 의도된 한계다
  (`:49-52`).

## 재검토 조건

- 쓰기 경로의 DDL이 동시성 병목으로 측정될 때 — 승격을 백그라운드/명시적 `og_reorganize`
  단계로 옮기는 안을 재평가한다.
- `text` 확장이 실운영에서 오탐(단 한 건의 이상 값 때문에 컬럼 전체가 text화)으로
  보고될 때 — 확장 대신 "해당 값만 `__ext`로 우회" 정책을 재검토한다.
- 배열 프로퍼티 승격이 필요해지면, `embedding` 이름 예약 또는 카탈로그 힌트로 `vector`
  선언을 보호하는 방법을 먼저 확보해야 한다.

<!-- affects: storage, catalog, cypher, vector -->
<!-- requires-update: docs/99_decisions/ADR-005-typed-property-columns.md -->
