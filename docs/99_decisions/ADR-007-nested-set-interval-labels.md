# ADR-007: 상속 판정을 nested-set 구간 라벨의 단일 범위 비교로 수행한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/002-ontology-type-system/plan.md` 기준일) |
| 영향 범위 | catalog, cypher, typeql, adapters |
| 근거 | `engine/src/catalog/labeling.rs:1-24`, `engine/sql/access.sql:40-52`, `.specify/memory/constitution.md` 원칙 IV, `specs/002-ontology-type-system/plan.md` Complexity Tracking |

> **이 문서가 답하는 질문**
> - `MATCH (v:Vehicle)`이 `Car`, `EV`까지 답하는 비용은 얼마인가?
> - 다중 상속(DAG)인데 nested-set을 어떻게 쓰는가?
> - `GAP = 1024`는 왜 필요한가?

## 배경

헌법 원칙 IV는 *"상속 질의는 상수 시간이어야 한다"* 고 요구하고, **런타임 재귀 CTE로
계층을 펼치는 방식을 금지**한다. 이는 헌법의 "금지 사항(Anti-Patterns)" 목록에도 별도
항목으로 올라 있다.

## 고려한 선택지

1. **런타임 재귀 CTE** — 구현이 가장 쉽다. 헌법이 명시적으로 금지. 계층이 깊어질수록
   모든 질의가 느려진다.
2. **전이 폐포(transitive closure) 물화** — 조회는 빠르나
   `specs/002-.../plan.md`가 기각: *"계층 변경 비용이 O(후손²)"*.
3. **비트셋 인코딩** — 기각 사유: *"타입 수 증가 시 폭이 선형 증가하고 스키마 변경 시
   전체 재작성"*.
4. **구간(nested-set/interval) 라벨링** — 타입마다 `(lft, rgt)`를 부여.

## 결정

**4안.** `og_catalog.type_label(graph_id, type_id, lft, rgt)`에 구간을 저장하고,
서브타입 판정을 다음 한 줄로 만든다 (`engine/src/catalog/labeling.rs:8`).

```text
    Y.lft <= X.lft  AND  X.rgt <= Y.rgt
```

**다중 상속은 타입당 라벨 행을 여러 개 두어 해결한다** — 루트에서 오는 경로마다 구간이
하나씩 생긴다. 그래서 `og_subtype_ids`가 `SELECT DISTINCT`로 시작한다
(`engine/sql/access.sql:44-52`).

라벨은 `GAP = 1024` 간격으로 발급된다 (`engine/src/catalog/labeling.rs:23`).

## 근거

- `engine/src/catalog/labeling.rs:1-13` 원문:
  > This module is the single reason `MATCH (v:Vehicle)` does not degrade as the
  > ontology grows. … one indexed range comparison — never a recursive CTE (which is
  > what spec 002 FR-010 forbids and SC-003 asserts against by inspecting EXPLAIN
  > output).
- 접근 함수 주석이 같은 주장을 반복하며 검증 지점을 못 박는다
  (`engine/sql/access.sql:40-42`): *"The label join below is THE constant-time
  inheritance test (spec 002 FR-009/010) — note the complete absence of recursion."*
- 다중 상속 처리와 GAP은 헌법 이탈로 정식 기록되어 있다
  (`specs/002-ontology-type-system/plan.md` Complexity Tracking):
  > 단일 nested-set은 트리만 표현 가능. DAG는 부모 경로마다 라벨이 필요
  > … GAP=1 은 삽입마다 전체 재할당 → 대형 온톨로지에서 수십 초
- 라벨 해소는 **컴파일 타임**에 일어난다. 그래서 실행 계획에 계층 순회가 아예 없다
  (`engine/src/cypher/compile.rs:9-10`).

## 결과

**긍정적**
- 서브타입 판정이 인덱스 범위 비교 1회. 계층 깊이와 무관하다.
- 같은 인덱스가 Cypher·TypeQL·RDF 매핑에 공통으로 쓰인다
  (`specs/010-.../plan.md` 설계 결정 4, `specs/006-.../plan.md` 매핑 표).

**부정적 / 감수한 대가**
- **라벨은 파생 데이터이며 계층 변경 시 재계산이 필요하다.** `og_relabel`이 그 진입점이다.
- 다중 상속 타입은 라벨 행이 여러 개이므로 공간이 늘고, 모든 조회가 `DISTINCT`를 요구한다.
- `GAP = 1024`는 공간 낭비다 — plan.md가 이를 이탈로 기록한다. 대신 같은 지점에 1024회
  삽입할 때까지 재할당이 없다.
- 한 지점의 삽입이 1024회를 넘으면 재할당이 발생하며, 그 순간은 상수 시간이 아니다.

## 재검토 조건

- 온톨로지가 **런타임에 자주 변형**되는 워크로드(예: 에이전트가 타입을 계속 만드는 경우)에서
  `og_relabel` 비용이 질의 이득을 잠식할 때 — 증분 재라벨링 또는 GAP 동적 조정을 재평가한다.
- 다중 상속 폭이 커져 라벨 행 수가 타입 수 대비 크게 증가할 때 — 경로별 라벨 대신
  구간 + 보조 비트맵 하이브리드를 다시 검토한다.

<!-- affects: catalog, cypher, typeql, adapters -->
