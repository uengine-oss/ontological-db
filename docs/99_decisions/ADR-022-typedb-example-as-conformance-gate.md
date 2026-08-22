# ADR-022: TypeDB 공식 예제를 **한 글자도 수정하지 않고** 통과시키는 것을 적합성 기준으로 삼는다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/010-typeql-query-surface/plan.md` 기준일) |
| 영향 범위 | typeql, testing, docs |
| 근거 | `specs/010-typeql-query-surface/spec.md` SC-001~SC-003·SC-010, `specs/010-.../plan.md` "Verification", `README.md` "Running a TypeDB example", `examples/typedb/bookstore/`, `tests/typeql/run.py` |

> **이 문서가 답하는 질문**
> - TypeQL 지원이 "된다"는 것을 무엇으로 증명하는가?
> - 자체 테스트 스위트로는 왜 부족한가?

## 배경

호환성 주장은 자기 자신을 테스트하면 언제나 통과한다. 우리가 만든 예제로 우리 파서를
검증하는 것은 **우리가 이해한 TypeQL**을 검증하는 것이지 TypeQL을 검증하는 것이 아니다.

이 문제는 Bolt 게이트웨이에서도 같은 형태로 제기되었고 같은 답을 얻었다
(`specs/011-.../plan.md`: *"우리가 만든 클라이언트로 우리 서버를 테스트하는 것은 증거가
아니다"*).

## 고려한 선택지

1. **자체 TypeQL 회귀 스위트** — 커버리지를 우리가 통제한다. 그러나 우리가 잘못 이해한
   부분은 스위트에도 잘못 들어간다.
2. **TypeDB TCK/공식 테스트 스위트** — 가장 엄밀하나 TypeQL 표면이 partial인 단계에서
   통과율이 의미를 갖기 어렵다.
3. **외부 산출물로 수용 판정** — TypeDB 공식 예제 저장소의 bookstore를 **바이트 그대로**
   가져와 적재·실행하고, 그 저장소 README에 결과가 함께 실린 질의로 대조한다.

## 결정

**3안.** `specs/010-.../plan.md` "Verification" 원문:
> 수용은 주장이 아니라 **외부 산출물**로 한다: TypeDB 공식 예제 저장소의 bookstore
> (`schema.tql`, `data.tql`, README에 결과가 함께 실린 질의)를 그대로 가져와 적재·실행하고
> 문서의 결과와 대조한다.

성공 기준이 spec에 수치로 박혀 있다.

| ID | 기준 |
|---|---|
| SC-001 | 공식 예제의 스키마 파일이 **한 글자도 수정하지 않고** 적재된다 |
| SC-002 | 데이터 파일도 **한 글자도 수정하지 않고** 전부 적재되며, 인스턴스 수가 원본 서술과 일치한다 |
| SC-003 | 예제 문서에 결과가 함께 실린 질의 중 함수를 쓰지 않는 것은 **100%** 문서와 같은 결과(순서 무시 집합 비교) |
| SC-010 | 지원/미지원 범위를 명시하고, **"부분은 부분"이라고 적는다** |

## 근거

- 예제는 벤더링되어 있고 그 사실이 README에 적혀 있다:
  > `schema.tql` and `data.tql` byte for byte as upstream publishes them. Nothing in
  > them was adjusted to run here.
- 회귀 스위트가 매 실행마다 이를 재검증한다:
  `python3 tests/typeql/run.py` — *"re-checks it, and 27 other properties, against the
  vendored files on every run."*
- **이 기준의 가치는 실패를 숨길 수 없다는 데 있다.** README가 그 실패를 그대로 적는다:
  > TypeDB **functions** (`fun`) are parsed, stored and reproduced by
  > `og_typeql_schema()`, but calling one raises an explicit error rather than guessing.
  > Two of the four queries in the bookstore README use functions, so two of four run
  > today. **That is the honest number.**
  자체 스위트였다면 "함수 미지원"은 스위트에서 빠졌을 것이고, 통과율은 100%로 보였을 것이다.
- SC-010이 미지원 항목을 **통과율에 포함하지 않고 미지원으로 표기**하도록 강제한다 —
  통과율을 부풀리는 가장 흔한 방법을 차단한다.

## 결과

**긍정적**
- 호환성 주장이 외부에서 검증 가능하다. 예제 파일은 업스트림 것이고 기대 결과는 업스트림
  문서 것이다.
- 미지원 항목이 "조용히 빠지는" 경로가 없다. 명시적 오류를 내고 문서에 적힌다.
- 같은 원칙이 Neo4j 쪽에도 적용된다 — 공식 Movie 샘플(`tests/neo4j-movies/`)과 Neo4j 자체
  MCP 서버(`examples/meeting-rooms/`)가 무수정으로 검증에 쓰인다.

**부정적 / 감수한 대가**
- **커버리지가 예제가 다루는 범위로 제한된다.** bookstore가 쓰지 않는 TypeQL 구문은
  이 게이트로 검증되지 않는다.
- 업스트림 예제가 바뀌면 우리 스위트가 깨진다 — 벤더링으로 시점을 고정하되, 갱신은 수동이다.
- "2/4 질의만 동작"이라는 낮은 숫자를 공개해야 한다. 마케팅상 불리하지만 그것이 이 결정의
  요점이다.

## 재검토 조건

- TypeQL 표면이 충분히 넓어지면(특히 `fun` 평가가 구현되면) **openCypher TCK에 대응하는
  TypeDB 적합성 스위트**로 기준을 격상할지 재평가한다. 헌법 원칙 X는 정확성 기준선으로
  openCypher TCK를 이미 지정하고 있으며, TypeQL 쪽에 대응물이 없는 것이 현재 공백이다.
- 예제 하나로는 커버되지 않는 의미론(부정, 재귀 함수, 스트림 파이프라인)이 사용자 이슈로
  반복 보고되면 기준 예제를 추가한다.

<!-- affects: typeql, testing, docs -->
<!-- requires-update: docs/99_decisions/ADR-015-two-query-languages.md, docs/99_decisions/ADR-023-benchmark-correctness-gate.md -->
