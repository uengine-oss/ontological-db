# ADR-010: `access.sql`의 접근 경로를 전부 `LANGUAGE sql`로 작성한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (spec 001 접근 경로 API 확정 시점) |
| 영향 범위 | storage, cypher, performance |
| 근거 | `engine/sql/access.sql:1-9`, `.specify/memory/constitution.md` 원칙 II, `engine/src/cypher/compile.rs:20-33` |

> **이 문서가 답하는 질문**
> - `og_expand`, `og_subtype_ids`, `og_vlp`가 왜 PL/pgSQL도 C도 아닌 `LANGUAGE sql`인가?
> - 이 선택이 실제로 무엇을 바꾸는가?

## 배경

접근 경로 함수(`og_expand`, `og_expand_batch`, `og_subtype_ids`, `og_nodes`, `og_edges`,
`og_vlp`, `og_reach_sql`, `og_prop` …)는 컴파일된 Cypher가 호출하는 최하층이다.
이 층이 **옵티마이저 장벽**이 되면 그 위의 모든 설계 주장이 무의미해진다 — 플래너가
그래프 패턴을 볼 수 있다는 것이 ADR-008의 핵심 근거이기 때문이다.

## 고려한 선택지

1. **PL/pgSQL 집합 반환 함수** — 작성이 편하다. 인라인되지 않으므로 플래너에게 블랙박스.
2. **C / Rust(pgrx) 집합 반환 함수** — 가장 빠른 단일 호출. 그러나 역시 인라인되지 않고,
   비용 추정이 상수 힌트(`ROWS`)에 의존한다.
3. **`LANGUAGE sql` 단순 집합 반환 함수** — PostgreSQL이 호출 질의로 인라인한다.

## 결정

**3안.** `engine/sql/access.sql`의 모든 함수를 `LANGUAGE sql STABLE PARALLEL SAFE`로
작성한다. 파일 헤더가 그 이유를 결정으로 못 박고 있다 (`engine/sql/access.sql:3-8`).

> Everything here is LANGUAGE SQL on purpose: PostgreSQL inlines simple set-returning
> SQL functions into the calling query, so the planner sees the adjacency scan itself —
> statistics, join order, parallelism and all. A PL/pgSQL or C set-returning function
> would be an optimisation barrier, which is precisely the mistake Constitution
> principle II forbids.

## 근거

- 위 주석 자체가 헌법 원칙 II와 직접 연결된 설계 근거다.
- **이 결정의 반례가 같은 저장소 안에 측정값으로 남아 있다.** `og_reach`는 Rust 집합 반환
  함수이고, 컴파일러 주석이 그 대가를 명시한다 (`engine/src/cypher/compile.rs:22-24`):
  > `og_reach` is a Rust set-returning function, so unlike `og_vlp` it does not inline
  > into the surrounding plan and it pays SPI setup per level — measured at a few tenths
  > of a millisecond.
  이 인라인 불가 비용이 ADR-013(보수적 적용)의 존재 이유 전체다.
- `docs/deep-traversal.md`는 순수 SQL 경로가 특정 형태에서 Rust 경로를 이긴다고 기록한다:
  체인 100,000홉에서 `og_reach_sql` 154.49ms 대 `og_reach` 1,015.93ms.
  *"`og_reach_sql()` is the better path whenever the frontier stays small and the depth
  is large, it is in `access.sql` for exactly that."*

## 결과

**긍정적**
- 컴파일된 Cypher에서 인접 스캔이 플래너에게 **평범한 조인**으로 보인다 (README "Look at
  the SQL" 예시의 `LATERAL unnest(adj3.nbr, adj3.eid)`).
- 병렬 질의·통계·`EXPLAIN`이 그래프 연산에도 그대로 적용된다.

**부정적 / 감수한 대가**
- 표현력이 SQL로 제한된다. 방문집합(visited set)처럼 상태가 필요한 알고리즘은 이 층에서
  표현할 수 없어 별도 Rust 함수(`og_reach`)로 나갔고, 그것은 인라인되지 않는다.
- `ROWS` 힌트(`ROWS 50`, `ROWS 500`, `ROWS 1000`)가 고정 상수다. 실제 차수 분포와 어긋나면
  계획이 나빠질 수 있다.
- **`og_reach_sql`은 존재하지만 컴파일러가 선택하지 않는다.** `docs/deep-traversal.md`가
  이유를 적는다: *"a third automatic choice would have to be made from a statistic that
  says whether frontiers overlap, and no such statistic is available for free."*

## 재검토 조건

- PostgreSQL이 집합 반환 함수의 인라인 조건을 완화하거나, 확장이 비용 함수를 제공할 수
  있게 되면 — Rust 경로도 플래너에 편입할 수 있는지 재평가한다.
- "프론티어가 겹치는가"를 저렴하게 알려주는 통계가 생기면, 컴파일러가
  `og_vlp` / `og_reach` / `og_reach_sql` 3지 선택을 하도록 ADR-013을 개정한다.

<!-- affects: storage, cypher, performance -->
<!-- requires-update: docs/99_decisions/ADR-013-conservative-bfs-rewrite.md -->
