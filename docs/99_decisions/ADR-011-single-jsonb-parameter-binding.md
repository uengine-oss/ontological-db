# ADR-011: 사용자 값을 SQL 텍스트로 보간하지 않고 단일 jsonb 파라미터로 바인딩한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (spec 003 구현 계획 기준일) |
| 영향 범위 | cypher, security, storage |
| 근거 | `engine/src/storage/mod.rs:42-46`, `engine/src/cypher/compile.rs:17-18`, `specs/003-cypher-query-engine/spec.md` FR-026, `engine/src/storage/traverse.rs:52-58` |

> **이 문서가 답하는 질문**
> - Cypher 파라미터와 프로퍼티 값이 어떻게 SQL에 도달하는가?
> - 주입(injection)은 어디서 막히는가?

## 배경

컴파일러가 Cypher를 SQL **텍스트**로 만든다는 것은(ADR-008) 그 텍스트에 사용자 값이 섞일
위험이 항상 존재한다는 뜻이다. spec 003 FR-026이 이를 요구사항으로 못 박는다:
> 시스템은 파라미터 바인딩을 지원해야 하며, 파라미터 값이 질의 구조를 변경할 수 없어야
> 한다(주입 방지).

## 고려한 선택지

1. **값을 이스케이프해 SQL 텍스트에 삽입** — 계획 캐시에 불리하고, 이스케이프 누락 한 곳이
   곧 취약점이다.
2. **값마다 별도 바인드 파라미터(`$1`, `$2`, …)** — 안전하지만 파라미터 개수가 질의마다
   달라져 컴파일 캐시 키가 복잡해진다.
3. **모든 사용자 값을 하나의 jsonb 파라미터에 담고, SQL은 그 안에서 꺼내 쓴다**

## 결정

**3안.** 컴파일러는 `PARAM = "$1"` 하나만 쓴다 (`engine/src/cypher/compile.rs:17-18`:
*"The bound jsonb parameter holding user `$params`."*).

쓰기 경로도 동일하다 (`engine/src/storage/mod.rs:42-46`):
> Declared properties become real columns; everything else is funnelled into `__ext`.
> All values are extracted from ONE bound jsonb parameter, so no user value is ever
> interpolated into SQL text (spec 003 FR-026).

생성되는 표현식은 `({param}->>'key')::{dtype}` 형태이며, 배열 컬럼은
`jsonb_array_elements_text` 를 거친다 (`engine/src/storage/mod.rs:208-217`).

## 근거

- 위 두 주석이 이 결정의 1차 근거이며 FR-026을 직접 인용한다.
- **SQL 텍스트에 들어가는 값은 열거된 집합으로 제한된다.** 예: 방향 문자는
  `'o' | 'i' | 'b'` 3원소 집합에 대해 검증된 뒤에야 포맷된다
  (`engine/src/storage/traverse.rs:52-58`):
  > The direction is validated against a three-element set before it reaches here, so
  > the format is not an injection path; the type ids still go in as a bound parameter,
  > because a thousand-element list rewritten per call would be its own cost.
- JSON 키는 `quote_json_key`로 작은따옴표를 이중화한다
  (`engine/src/storage/mod.rs:224-226`).
- 부수 효과로 **컴파일 캐시가 성립한다**. `(graph, query)` → SQL 캐시가 가능한 이유는 값이
  SQL 텍스트에 들어가지 않기 때문이다 (`specs/003-.../plan.md` "컴파일 캐시").

## 결과

**긍정적**
- 파라미터 값이 질의 구조를 바꿀 수 없다 (FR-026 충족).
- 같은 Cypher 텍스트가 값과 무관하게 같은 SQL로 컴파일되므로 컴파일 캐시와 PostgreSQL 계획
  캐시가 둘 다 동작한다.

**부정적 / 감수한 대가**
- 모든 값이 jsonb를 통과하므로 **타입 정보가 한 번 소실되고 캐스팅으로 복원된다.**
  그래서 컴파일러에 타입 힌트 기구가 필요하다 (`specs/003-.../plan.md` "컴파일 예시" 및
  `plan.md` 아키텍처 표: *"타입 힌트 기반 파라미터 캐스팅"*).
- jsonb 추출·캐스팅이 실행 시 비용이며, README가 인정한 Cypher 표면 오버헤드의 일부다.
- 식별자(테이블명, 컬럼명, 타입 id 배열)는 여전히 SQL 텍스트로 만들어진다. 이들은 사용자
  값이 아니라 **카탈로그에서 온 값**이라는 전제 위에 안전하다 — 이 전제가 깨지면 취약점이
  된다.

## 재검토 조건

- 카탈로그에서 오는 식별자가 사용자 입력으로 직접 만들어지는 경로가 생기면(예: 임의 문자열
  타입명 생성 API), `quote_ident` 적용 범위를 전면 재감사해야 한다.
- jsonb 캐스팅 비용이 Cypher 표면 오버헤드의 지배적 항목으로 측정되면, 값 개수가 적은
  질의에 한해 개별 바인드 파라미터를 쓰는 하이브리드를 재평가한다.

<!-- affects: cypher, security, storage -->
