# ADR-008: Cypher를 최상위 문장 문법이 아니라 함수 호출(`og_cypher`)로 진입시킨다

| 항목 | 값 |
|---|---|
| 상태 | Accepted (헌법 원칙 II 부분 미달로 기록됨) |
| 날짜 | 2026-08-06 (`specs/003-cypher-query-engine/plan.md` 기준일) |
| 영향 범위 | cypher, api, bolt, interop |
| 근거 | `specs/003-cypher-query-engine/plan.md` Complexity Tracking, `.specify/memory/constitution.md` 원칙 I·II, `README.md` "Governance" 문단, `engine/src/cypher/compile.rs:1-10` |

> **이 문서가 답하는 질문**
> - 헌법이 "Cypher를 문자열 인자로 받는 함수 인터페이스"를 금지했는데 왜 `og_cypher('g', $$…$$)` 인가?
> - 그럼 Apache AGE와 무엇이 다른가?

## 배경

헌법 원칙 II는 Apache AGE 방식 — Cypher를 문자열에 가두고 결과를 `agtype`으로 반환 — 을
**명시적으로 거부**한다. 이유는 셋이다: 옵티마이저가 패턴 내부를 못 본다, 파라미터
바인딩이 무력화된다, 계획 캐시가 무력화된다.

원칙 II를 문자 그대로 달성하려면 PostgreSQL의 **최상위 문장 문법**을 확장이 교체해야 한다.
PostgreSQL 16에는 그런 지원 훅이 없다.

## 고려한 선택지

1. **raw parser 교체 (커널 패치)** — 원칙 II를 문자 그대로 달성. 그러나 원칙 I
   (NON-NEGOTIABLE, ADR-001)과 정면 충돌한다.
2. **SQL/PGQ 스타일 SQL 내장 문법(`GRAPH_TABLE`)** — 표준적이나 v1 범위 밖.
3. **함수 호출 진입 + 컴파일 결과를 평범한 SQL로 노출**

## 결정

**3안.** 진입점은 `og_cypher('graph', $$ … $$)` 함수 호출로 유지한다.
헌법 이탈로 `specs/003-cypher-query-engine/plan.md` Complexity Tracking에 기록되어 있으며,
README도 이를 전면에 적는다:
> Cypher still enters through a function call because PostgreSQL 16 has no hook to
> replace the top-level parser without patching the kernel. Constitution principle I
> (never fork) outranks principle II, so principle II loses this round, and the plan
> says so.

## 근거

- `plan.md` Complexity Tracking 원문:
  > PostgreSQL 16에는 최상위 문장 문법을 확장이 교체할 수 있는 지원 훅이 없다. raw parser
  > 교체는 커널 패치를 요구하며 원칙 I(NON-NEGOTIABLE)과 정면 충돌한다. 원칙 I이 이긴다
- **AGE와의 차이는 진입 형태가 아니라 그 뒤에 무엇이 일어나는가에 있다.** 같은 표가
  이어서 기록한다:
  > 그러나 AGE와 달리 **원칙 II의 실질(옵티마이저 가시성, 파라미터 바인딩, 계획 캐시,
  > 표준 타입 반환)은 모두 달성**했다.
- 컴파일러 주석이 그 실질을 설명한다 (`engine/src/cypher/compile.rs:3-8`):
  > The output is ordinary SQL over ordinary relations. That is the whole point:
  > PostgreSQL's cost-based optimiser gets to choose the join order, the scan methods
  > and the parallelism for the graph pattern, using real statistics on real tables.
- 이 주장은 검증 가능하다 — `og_cypher_sql()`이 컴파일된 SQL을 그대로 내주므로 사용자가
  그것을 뷰·CTE·조인에 직접 넣을 수 있다 (README "Look at the SQL", Studio의 SQL 탭).

## 결과

**긍정적**
- 원칙 I을 지키면서 원칙 II의 실질적 이득(플래너 가시성, 실컬럼 술어, 파라미터 바인딩)을
  얻는다.
- 컴파일된 SQL이 공개되므로 주장이 감사 가능하다.
- 같은 진입점 하나를 Bolt 게이트웨이가 재사용한다 — 의미론이 두 벌로 갈라지지 않는다
  (ADR-017).

**부정적 / 감수한 대가**
- `psql`에서 Cypher를 그대로 칠 수 없다. 항상 함수 호출로 감싸야 한다.
- 결과가 jsonb 객체이므로 **`RETURN` 절의 필드 순서가 소실된다.** Bolt 게이트웨이는 이를
  위해 `og_cypher_columns(query)`를 별도로 요구했다
  (`specs/011-bolt-protocol-gateway/plan.md` 설계 결정 3).
- jsonb 투영 비용이 실측으로 확인된다. README: 3홉에서 Cypher 표면 33.86ms 대 저장 경로
  4.33ms — *"7.8× over the raw storage path … mostly jsonb projection and SPI
  round-tripping. That is the honest current price of the query surface."*

## 재검토 조건

- **PostgreSQL이 상위 파서 대체 훅을 제공하면** 즉시 재평가한다. 이것이 이 ADR의 1순위
  트리거다.
- SQL/PGQ(`GRAPH_TABLE`)를 구현해 SQL 문장 안에서 그래프 패턴을 쓸 수 있게 되면, 이 ADR은
  "함수 호출은 두 진입점 중 하나"로 재작성된다 (`plan.md`가 v2 계획으로 명시).
- jsonb 투영 비용이 Cypher 표면의 지배적 비용으로 남는 한, 반환 형태(컴포지트/레코드)
  변경은 이 ADR과 별개로 진행 가능하다.

<!-- affects: cypher, api, bolt, interop -->
<!-- requires-update: docs/99_decisions/ADR-001-postgresql-extension-not-fork.md, docs/99_decisions/ADR-017-bolt-gateway-separate-process.md -->
