# ADR-016: TypeQL 관계를 간선이 아니라 노드로 물화(reify)한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/010-typeql-query-surface/plan.md` 기준일) |
| 영향 범위 | typeql, storage, catalog, cypher |
| 근거 | `specs/010-typeql-query-surface/plan.md` 설계 결정 2·3, Complexity Tracking, "Data model mapping" 표 |

> **이 문서가 답하는 질문**
> - TypeDB의 `authoring (work: $book, author: $author)`가 저장소에서 무엇이 되는가?
> - 왜 `(src, dst)` 간선으로 표현하지 않았는가?

## 배경

TypeDB의 관계는 속성 그래프의 간선과 **다른 종류의 것**이다.

- 3항 이상일 수 있다 (`authoring (work:, author:, ...)`)
- 자신이 속성을 소유한다 (`relation has attribute`)
- 다른 관계의 역할을 수행할 수 있다

`(src, dst)` 두 끝점을 갖는 간선은 이 셋 중 어느 것도 온전히 표현하지 못한다.

## 고려한 선택지

1. **간선으로 표현** — 2항 관계만 되고 관계의 속성 소유가 표현되지 않는다.
2. **간선 + `og_role_player` 혼합** — `plan.md` Complexity Tracking이 기각:
   *"'관계가 속성을 소유'를 표현하지 못한다."*
3. **관계를 노드로 물화** — 관계 인스턴스가 `og_data.og_node` + 타입 테이블 1행이 되고,
   역할 배정은 `og_data.og_role_player`에 들어간다.

## 결정

**3안.** 매핑은 다음과 같다 (`specs/010-.../plan.md` "Data model mapping").

| TypeQL 개념 | 저장 |
|---|---|
| relation 타입 | `og_catalog.type` kind `r` + **노드** 테이블 `og_data.n_<tid>` (물화) |
| `relates r` | `og_catalog.role (rel_type_id, name, ordinal)` |
| 역할 배정 | `og_data.og_role_player (edge_id=관계노드, role_id, player_id)` |
| attribute 인스턴스 | `og_data.og_node` + `og_data.a_<tid> (id, val UNIQUE)` — 값 UNIQUE로 자동 공유 |
| 소유 (`has`) | 그래프당 하나의 내부 관계 타입 `$has`의 간선 |

**역할 배정에는 인접 세그먼트를 쓰지 않는다.** `og_role_player`가 spec 002가 바로 이
목적으로 만든 구조이며 PK와 역인덱스를 이미 갖고 있기 때문이다.

## 근거

- `plan.md` 설계 결정 2 원문:
  > TypeDB 관계는 3항 이상일 수 있고, 속성을 소유하며, 다른 관계의 역할을 수행한다.
  > 간선(src,dst)으로는 이 셋 중 어느 것도 온전히 표현되지 않는다.
- 속성 공유(값 중복 제거)가 의미론의 핵심임을 설계 결정 3이 밝힌다:
  > 속성은 값으로 중복 제거된 인스턴스다. TypeDB 의미론의 핵심이며, 이것이 있어야
  > "두 책이 같은 장르를 공유한다"가 그래프 탐색으로 답해진다.
  소유는 `og_adj` 위의 간선이므로 확장이 순차 읽기가 된다 (ADR-004의 이득을 그대로 받는다).
- 속성 타입 필터가 공짜인 이유는 ADR-003의 ID 인코딩이다:
  > 식별자의 18비트에 type_id가 박혀 있으므로, `has genre $g`의 장르 필터는 이웃 id에 대한
  > 시프트-마스크다. 카탈로그 조인도, 별도 인덱스도 필요 없다.
- 이 매핑은 **숨기지 않는다.** `plan.md` FR-040/043: *"이는 숨겨진 사실이 아니라
  문서화된 투영이다."* README도 같은 취지로 `og_typeql_attribute` / `og_typeql_role`을
  가리킨다.

## 결과

**긍정적**
- n항 관계, 관계의 속성 소유, 관계의 역할 수행이 모두 표현된다.
- 속성 값 공유가 저장 구조로 성립하므로 "같은 장르를 가진 책들"이 그래프 탐색이 된다.
- Cypher와 TypeQL이 같은 저장소를 본다 (ADR-015).

**부정적 / 감수한 대가**
- **Cypher 사용자에게 물화된 관계 노드와 `$has` 간선이 보인다.** 순수한 속성 그래프 뷰가
  아니다.
- 역할 배정이 `og_role_player`에 있으므로 인접 세그먼트의 순차 읽기 이득을 받지 못한다.
  `plan.md`가 이를 **미구현 최적화로 명시**한다:
  > CSR 세그먼트로 옮기는 최적화는 이식성이 성립한 뒤의 과제로 남긴다 — **미구현임을 명시**
- 관계 하나가 노드 1개 + 역할 수만큼의 `og_role_player` 행을 만든다. 2항 관계만 쓰는
  워크로드에서는 간선보다 무겁다.
- 카탈로그 테이블이 하나 늘었다 (`og_catalog.typeql_function`) — 함수 본문이 기존 어느
  테이블에도 맞지 않았기 때문이며, Complexity Tracking에 기록되어 있다.

## 재검토 조건

- 역할 배정 조회가 TypeQL 질의의 지배적 비용으로 측정되면, `og_role_player`를 CSR 세그먼트로
  옮기는 최적화를 착수한다 (plan.md가 이미 조건부로 예고한 작업).
- 2항 관계만 쓰는 TypeQL 워크로드가 주 사용처가 되면, 2항 관계를 간선으로 최적화하는
  하이브리드를 재평가한다 — 단, 관계의 속성 소유가 나타나는 순간 물화로 되돌아가야 하므로
  전환 비용이 크다.

<!-- affects: typeql, storage, catalog, cypher -->
<!-- requires-update: docs/99_decisions/ADR-015-two-query-languages.md -->
