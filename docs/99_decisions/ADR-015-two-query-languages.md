# ADR-015: 하나의 그래프 위에 Cypher와 TypeQL 두 질의 언어를 제공한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/010-typeql-query-surface/plan.md` 기준일) |
| 영향 범위 | typeql, cypher, catalog, api |
| 근거 | `specs/010-typeql-query-surface/plan.md` Summary·Complexity Tracking, `.specify/memory/constitution.md` 원칙 II·IV·VI, `README.md` "Running a TypeDB example" |

> **이 문서가 답하는 질문**
> - 헌법이 "중심 언어는 언제나 Cypher"라고 했는데 왜 두 번째 언어를 넣었는가?
> - TypeQL을 Cypher로 번역하지 않은 이유는?

## 배경

헌법 원칙 IV는 타입 시스템을 설계하면서 **TypeDB의 개념 모델을 명시적으로 벤치마킹**한다:
entity/relation(+role)/attribute 타입과 그들 사이의 서브타입 관계, 다중 상속, role 상속.
spec 002가 그 모델을 실제로 구현했다.

그런데 원칙 VI는 *"중심 언어는 언제나 Cypher다"* 이고, 원칙 II는 Cypher를 1급으로 못 박는다.
개념 모델은 가져왔는데 그 모델의 언어는 거부한 상태 — 이것이 이 결정의 출발점이다.

## 고려한 선택지

1. **TypeQL을 지원하지 않음** — 원칙 II에 가장 충실. 그러나 TypeDB 사용자가 이식할 경로가
   없다.
2. **TypeQL → Cypher 번역** — 파서 하나만 추가하면 된다. `plan.md`가 기각:
   *"관계 1급성·역할·속성 공유에서 반드시 새며, 조용한 오답을 만든다."*
3. **독립 렉서·파서·컴파일러를 두되, 카탈로그와 저장 구조를 공유**

## 결정

**3안.** `engine/src/typeql/`에 독립된 lexer/parser/ast/compile/schema/write/dump를 둔다.
공유하는 것은 AST가 아니라 **카탈로그(`og_catalog.*`)와 저장 구조(`og_data.*`)** 다.

`specs/010-.../plan.md` 설계 결정 1:
> TypeQL은 Cypher 위에 얹지 않는다. 별도의 렉서·파서·컴파일러를 둔다. TypeQL을 Cypher로
> 번역하는 경로는 관계의 1급성·역할·속성 공유 의미론에서 반드시 새기 때문이다. 공유하는
> 것은 AST가 아니라 **카탈로그와 저장 구조**다.

## 근거

- 헌법 이탈은 정식 기록되어 있다 (`specs/010-.../plan.md` Complexity Tracking):
  > 두 번째 질의 언어 추가 (원칙 II) | 002가 TypeDB 개념 모델을 채택했으면서 그 언어를
  > 거부한 것은 이식성 측면에서 절반의 결정이었다
- 원칙 VI("코어는 하나, 표준은 어댑터로")는 **저장 엔진과 타입 시스템이 하나**일 것을
  요구하지, 언어가 하나일 것을 요구하지 않는다. 이 결정은 그 조건을 지킨다 —
  이중 저장 구조는 없다.
- 결과가 README에서 검증 가능하다: TypeQL로 적재한 bookstore 그래프를 Cypher로 그대로
  질의할 수 있다.
  ```sql
  SELECT og_cypher('bookstore',
    $$ MATCH (b:ebook)-[:`$has`]->(t:title) RETURN t.val $$);
  ```
  그리고 그 매핑은 숨겨지지 않는다 — `og_typeql_attribute`, `og_typeql_role`에서 읽을 수
  있다 (`plan.md` FR-040/043: *"이는 숨겨진 사실이 아니라 문서화된 투영이다"*).

## 결과

**긍정적**
- TypeDB 애플리케이션이 이식 가능하다 (ADR-022의 적합성 기준으로 검증).
- 상속·다형성이 두 언어에서 같은 구간 라벨 인덱스로 풀린다 (ADR-007).
- 하나의 그래프, 하나의 트랜잭션, 하나의 권한/감사 경로.

**부정적 / 감수한 대가**
- **파서·컴파일러가 두 벌이다.** `engine/src/typeql/`만 4,428줄 규모이며, Cypher 쪽과
  독립적으로 유지보수해야 한다.
- 두 언어의 의미론 차이가 저장 계층에 노출된다 — 물화된 관계 노드와 `$has` 간선이
  Cypher 사용자 눈에 보인다.
- TypeQL 표면은 **partial**이다. 사용자 정의 함수(`fun`)는 파싱·저장·재현되지만 호출은
  명시적 오류다. README가 *"Two of the four queries in the bookstore README use
  functions, so two of four run today. That is the honest number."* 로 적는다.

## 재검토 조건

- 두 컴파일러의 중복이 유지보수 비용을 지배하기 시작하면, **공용 논리 계획**을 도입할지
  재평가한다. 참고로 SPARQL은 이미 그 길을 택했다 —
  `specs/006-semantic-web-adapters/plan.md`: *"SPARQL 파서는 자체 AST를 만들지 않고
  `cypher::ast::Query` 로 lower 한다."* TypeQL이 그렇게 하지 않은 것은 의미론 손실 때문이며,
  그 판단이 바뀌어야만 재검토가 성립한다.
- ISO GQL 정렬이 진행되어 Cypher 쪽 AST가 관계 1급성을 표현할 수 있게 되면, 공유 계획의
  가능성이 다시 열린다.

<!-- affects: typeql, cypher, catalog, api -->
<!-- requires-update: docs/99_decisions/ADR-016-typeql-relations-reified-as-nodes.md, docs/99_decisions/ADR-022-typedb-example-as-conformance-gate.md -->
