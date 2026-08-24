# ADR-009: 읽기 경로는 SQL 생성으로, 쓰기 경로는 Rust SPI로 이원화한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/003-cypher-query-engine/plan.md` 기준일) |
| 영향 범위 | storage, cypher, typeql |
| 근거 | `engine/src/storage/mod.rs:1-10`, `specs/003-cypher-query-engine/plan.md` Complexity Tracking, `specs/010-typeql-query-surface/plan.md` 설계 결정 5 |

> **이 문서가 답하는 질문**
> - 왜 읽기와 쓰기가 서로 다른 방식으로 구현되어 있는가?
> - 쓰기를 단일 SQL로 표현하지 않은 이유는?

## 배경

Cypher/TypeQL은 읽기(`MATCH … RETURN`)와 쓰기(`CREATE`/`MERGE`/`SET`/`DELETE`,
`insert`/`put`/`delete`)를 같은 언어 안에 담는다. 그러나 두 경로의 요구가 다르다.

- 읽기는 **플래너가 패턴 전체를 봐야** 한다 (ADR-008의 근거 전체).
- 쓰기는 **한 트랜잭션 안에서 세 구조를 동시에 정합하게** 유지해야 한다:
  레지스트리(`og_node`/`og_edge`), 타입별 프로퍼티 테이블, 그리고 인접 세그먼트의
  **양방향**(001 FR-012).

## 고려한 선택지

1. **전부 SQL 생성** — 쓰기도 하나의 SQL로 표현. `plan.md`가 기각:
   *"단일 SQL로 표현하면 트리거나 CTE 부작용에 의존하게 되어 검증이 어려워진다."*
2. **전부 Rust SPI(함수 파이프라인)** — 읽기까지 함수 안으로 들어가면 플래너가 패턴을
   못 본다. 이는 정확히 AGE가 한 실수다.
3. **읽기=SQL 생성 / 쓰기=Rust 절차적 실행**

## 결정

**3안.** `engine/src/storage/mod.rs`가 이 경계를 모듈 주석으로 선언한다 (`:1-10`).

> Write paths live here in Rust because they must keep three structures in lock-step
> inside one transaction (spec 001 FR-012): the registry, the typed property table, and
> both directions of the adjacency segment.
>
> Read paths are deliberately **not** here. The Cypher compiler emits SQL that touches
> `og_data.og_adj` directly so the PostgreSQL planner sees the whole traversal — that is
> the difference between this design and a function-call pipeline the optimiser cannot
> look into.

TypeQL도 같은 원칙을 따른다 (`specs/010-.../plan.md` 설계 결정 5):
*"읽기는 하나의 SQL로 컴파일한다 … 쓰기는 바인딩마다 절차적으로 실행한다."*

## 근거

- 쓰기의 절차적 실행은 헌법 이탈로 정식 기록되어 있다
  (`specs/003-.../plan.md` Complexity Tracking):
  > 001 FR-012가 인접 양방향 갱신을 같은 트랜잭션에 요구. 단일 SQL로 표현하면 트리거나
  > CTE 부작용에 의존하게 되어 검증이 어려워진다
- 읽기가 SQL이어야 하는 이유는 컴파일러 주석과 `access.sql` 헤더가 반복해서 말한다
  (ADR-008, ADR-010).
- 이 경계가 실제로 지켜지는지 확인 가능하다: `og_cypher_sql()`이 읽기 질의의 전체 SQL을
  내주고, 쓰기 질의는 `og_cypher_check()`가 `w`로 분류한다.

## 결과

**긍정적**
- 읽기: 플래너가 조인 순서·스캔 방식·병렬성을 실제 통계로 고른다.
- 쓰기: 세 구조의 정합성이 한 트랜잭션 안에서 명시적으로 보장된다. 트리거 부작용에 의존하지
  않으므로 검증 가능하다 (원칙 IX).

**부정적 / 감수한 대가**
- **두 개의 코드 경로.** 같은 의미론(예: 프로퍼티 타입 캐스팅)이 두 곳에 존재할 수 있다.
- 쓰기가 바인딩마다 절차적으로 실행되므로 **대량 쓰기 성능이 자연히 나쁘다.**
  `plan.md`는 이를 인정하고 별도 대응을 명시한다: *"대량 쓰기 성능은
  `og_create_node`/`og_create_edge` 벌크 경로로 별도 대응"*.
- 쓰기 경로가 SPI 호출을 반복하므로 트랜잭션 길이가 길어질 수 있다.

## 재검토 조건

- 쓰기 처리량이 제품 병목으로 측정될 때 — 벌크 경로 확대, 또는 인접 갱신을 배치화하되
  **같은 트랜잭션 안에서** 수행하는 방식을 재평가한다. (헌법 원칙 IX가 비동기 인접 갱신을
  금지하므로 "나중에 갱신"은 선택지가 아니다.)
- 읽기 컴파일러가 커버하지 못하는 절이 늘어나 읽기 일부가 Rust로 넘어오기 시작하면,
  그 시점이 이 경계가 무너지는 신호다.

<!-- affects: storage, cypher, typeql -->
