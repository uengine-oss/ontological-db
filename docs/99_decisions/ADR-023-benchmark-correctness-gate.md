# ADR-023: 벤치마크에 정확성 게이트를 두고, 결과가 다르면 성능 수치를 무효 처리한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/009-benchmark-conformance/plan.md` 기준일) |
| 영향 범위 | testing, docs, performance |
| 근거 | `specs/009-benchmark-conformance/plan.md` 핵심 설계 결정 2, `specs/009-.../spec.md` SC-005·라인 183/209, `docs/benchmark.md` "Method", `docs/deep-traversal.md` "Method", `.specify/memory/constitution.md` 원칙 X |

> **이 문서가 답하는 질문**
> - 벤치마크 수치를 어떻게 믿을 수 있는가?
> - 시스템들이 서로 다른 답을 내면 어떻게 되는가?

## 배경

헌법 원칙 X는 *"성능 주장에는 재현 가능한 벤치마크가 동반되어야 한다"* 고 요구한다.
그러나 재현 가능성만으로는 부족하다 — **서로 다른 질문에 답하는 시스템들을 나란히 놓으면**
재현 가능하게 틀린 숫자가 나온다.

## 고려한 선택지

1. **성능만 측정** — 가장 흔한 형태. `plan.md`가 한 문장으로 기각:
   *"이것이 없는 벤치마크는 마케팅이다."*
2. **결과를 사람이 눈으로 확인** — 규모가 커지면 지켜지지 않는다.
3. **하네스가 결과를 자동 대조하고, 불일치 시 해당 타이밍을 무효 처리**

## 결정

**3안.** `specs/009-.../plan.md` 핵심 설계 결정 2:
> **정확성 먼저.** 시스템 간 결과가 다르면 성능 수치를 **무효 처리**한다. 이것이 없는
> 벤치마크는 마케팅이다.

spec의 성공 기준으로도 박혀 있다 (SC-005): *"비교 실행에서 시스템 간 결과 정확성 검증이
**자동으로 수행**되며, 불일치 시 성능 수치가 무효 처리된다."*

## 근거

- `docs/benchmark.md` "Method"가 이 게이트를 **하네스 자신에게 적대적**이라고 표현한다:
  > The harness is written to be hostile to its own conclusions.
  > … **Answers are checked before timings are reported.** … This is not ceremony — an
  > earlier version of the harness caught the systems starting from *different* nodes
  > because the start property was not unique, and every number before that fix was
  > meaningless.
- 게이트가 실제로 여러 번 작동했고, 그 결과가 문서에 **정정 이력**으로 남아 있다.

  | 정정 | 내용 |
  |---|---|
  | AGE 615× 주장 철회 | *"An earlier version of this README claimed 615× against AGE; that number was measured against an AGE with no index on its edge endpoints, and it should not have been published."* (`README.md`) |
  | Neo4j 1.8ms 철회 | *"An earlier draft of this document had Neo4j at 1.8 ms for every query; that number was the driver warming up."* (`docs/benchmark.md`) |
  | 질문 정규화 | Cypher와 pgGraph가 시작 노드 포함 여부에서 달라 비교가 무효화되므로, 워크로드를 "시작 노드를 제외한 k홉 내 distinct 노드"로 바꾸고 그 대가(AGE에 약 90ms 페널티)를 **공개**했다 (`docs/deep-traversal.md`) |
  | "5홉 이후 Neo4j보다 빠르다" 정정 | *"A correction we owe the earlier section"* — 포화되는 형태에서만 참이며, 직경이 있는 그래프에서는 Neo4j가 이긴다 (`docs/deep-traversal.md`) |

- 게이트는 코드로 존재한다: `bench/csr/deep.py`가 답을 JSON 출력에 기록하고 불일치 시
  **non-zero로 종료**한다 (`docs/deep-traversal.md` "Method"). README도
  *"The harness voids its own timings when the systems disagree on an answer."* 로 적는다.
- 게이트는 크래시도 다룬다: AGE 백엔드가 `signal 9`로 죽으면 같은 실행의 다른 시스템 수치도
  무효가 되므로, 하네스가 크래시를 감지하고 그 시스템에 더 깊은 질의를 하지 않으며 복구를
  기다린 뒤 계속한다 (`docs/deep-traversal.md`).
- 공정성 조건도 함께 규정되어 있다: 각 시스템에 **유능한 운영자가 만들 인덱스**를 준다.
  AGE는 설치 상태로 벤치마킹하면 "미설정을 측정"하는 것이 되므로 문서가 요구하는 인덱스 3개를
  준다 — 그 결과 AGE 1홉이 29.6ms → 1.6ms가 되었다 (`docs/benchmark.md`).

## 결과

**긍정적**
- 발표된 수치가 감사 가능하다. 원자료가 `bench/results/*.json`에 커밋되어 있고,
  Studio의 벤치마크 리포트가 그 파일에서 직접 렌더링되므로 *"the page and the measurements
  cannot disagree"* (README).
- 프로젝트가 자기에게 불리한 결과를 스스로 발표하게 된다 — 재귀 CTE가 여러 지점에서 우리를
  이긴다는 사실이 README 본문에 있다.

**부정적 / 감수한 대가**
- **질문을 모든 시스템이 동일하게 표현할 수 있는 형태로 정규화해야 한다.** 그 정규화 비용이
  시스템마다 다르며(AGE +90ms), 그것을 공개하는 것으로만 상쇄된다.
- 하네스 복잡도가 커진다 (`bench/harness.py` 53KB).
- 한 시스템이 답을 못 내면 그 깊이 이후의 비교가 성립하지 않아 표에 `—`가 늘어난다.

## 재검토 조건

- 정규화가 불가능한 질문(각 시스템의 의미론이 근본적으로 다른 경우)이 워크로드에 필요해지면,
  "무효 처리" 대신 **의미론 차이를 명시한 별도 표**로 보고하는 규칙을 추가해야 한다.
- CI 회귀 게이트(P3)가 도입되면, 정확성 불일치가 **병합 차단** 사유가 되도록 승격한다
  (헌법 "품질 게이트"가 이미 요구하는 형태).
- LDBC SNB(P2)가 도입되면 참조 결과와의 대조가 시스템 간 대조를 대체하거나 보완할 수 있다.

<!-- affects: testing, docs, performance -->
<!-- requires-update: docs/99_decisions/ADR-022-typedb-example-as-conformance-gate.md -->
