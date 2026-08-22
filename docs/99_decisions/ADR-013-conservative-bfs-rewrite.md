# ADR-013: 방문집합 BFS 재작성을 보수적으로만 적용한다 (관측 가능성 + 손익분기 깊이)

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-17 (커밋 `f9e64fd`, 규칙 개정 `7d60c82`) |
| 영향 범위 | cypher, performance, correctness |
| 근거 | `engine/src/cypher/compile.rs:20-77`(`prefer_reachability`), `:318-349`(`multiplicity_blind`), `docs/deep-traversal.md` "When the compiler rewrites, and when it must not", `engine/tests/sql/05_reachability.sql` |

> **이 문서가 답하는 질문**
> - 어떤 질의가 BFS로 재작성되고 어떤 질의가 안 되는가?
> - 왜 무조건 재작성하지 않는가?
> - 손익분기 규칙은 어떻게 정해졌는가?

## 배경

ADR-012의 재작성은 **답을 바꿀 수 있는** 변환이다. `count(b)`는 걷기를 세고
`count(DISTINCT b)`는 노드를 세므로, 관측 가능성을 잘못 판단하면 성능이 아니라 **정답**이
틀린다. 동시에 `og_reach`는 인라인되지 않고 레벨당 SPI 비용을 내므로, 얕은 질의에 적용하면
**더 느려진다** — 실제로 첫 구현이 2홉 질의를 느리게 만들었다.

## 고려한 선택지

1. **무조건 재작성** — 첫 버전. 2홉 질의가 느려졌다(`README.md` "Past three hops":
   *"the first version that applied it unconditionally made two-hop queries slower"*).
2. **사용자 힌트로 선택** — 호환성 목표(Neo4j 앱 무수정)와 충돌.
3. **두 개의 독립 게이트: (a) 다중도 관측 불가 + (b) 손익분기 통과**

## 결정

**3안.** 두 게이트를 모두 통과해야 `og_reach`로 내린다
(`engine/src/cypher/compile.rs:865`).

### 게이트 (a) — 다중도를 관측할 수 없는가 (`multiplicity_blind`, `:339`)

| 조건 | 판정 |
|---|---|
| `WITH`가 질의 어디에든 있음 | **재작성 안 함** (RETURN 전에 집계 가능, 이 패스는 내부를 보지 않음) |
| `RETURN DISTINCT …` | 재작성 |
| 그 외 | 투영이 집계여야 하고, **모든** 집계가 중복에 둔감해야 함 |
| `count(DISTINCT x)`, `collect(DISTINCT x)`, `min`, `max` | 둔감 → 허용 (`blind_expr`, `:80-100`) |
| `count(x)`, `sum`, `avg`, 사용자 정의 | 중복에 민감 → **재작성 안 함** |
| 경로 변수(`MATCH p = …`) 또는 관계 변수(`-[e:K*1..3]->`) 바인딩 | RETURN과 무관하게 **홉 단계에서 거부** |

### 게이트 (b) — 손익이 맞는가 (`prefer_reachability`, `:34`)

- `Σ degreeⁱ > WALKS(=512)` 이면 재작성. 차수는 `pg_class.reltuples`(노드/엣지)에서
  읽는다 — **스캔이 아니라 카탈로그 조회**다 (`:46-51`).
- 통계가 없는(ANALYZE 안 된) DB는 깊이만으로 판단: `max >= DEEP(=4)` (`:44`, `:52`).

## 근거

- 좁게 만든 이유가 주석에 명시되어 있다 (`engine/src/cypher/compile.rs:320-321`):
  *"The test is deliberately narrow, because being wrong here changes answers rather
  than timings."*
- 임계값이 낮은 이유는 **두 실패 모드가 비대칭**이기 때문이다 (`:36-41`):
  > enumerating when we should not have runs out of time or memory — 2.7 s at twenty
  > hops on a lattice, 90 s at thirty — while reaching when we should not have costs a
  > bounded fraction of a millisecond. A rule this cheap should err toward the bounded
  > loss.
- **첫 번째 규칙은 틀렸고, 격자가 그것을 증명했다** (`:60-67`, `docs/deep-traversal.md`
  "The cost rule was wrong, and the lattice proved it"):
  > on a 1000x1000 lattice ten hops is 2,046 walks against a million nodes —
  > "affordable" by that rule — but only 66 nodes are reachable, and enumerating cost
  > 3.83 ms against 0.30 ms. Degree alone cannot see that overlap, so the rule no
  > longer pretends to; it asks only whether enough walks are coming to pay for the
  > switch.
  (문서와 주석의 수치가 4.29ms/0.28ms와 3.83ms/0.30ms로 미세하게 다르다 — 서로 다른 실행의
  중앙값이며, 결론은 동일하다.)
- 손익분기가 측정과 일치함이 확인되었다 (`docs/deep-traversal.md`): dense 픽스처에서
  깊이 3은 8,420 walks로 "아니오", 깊이 4는 168,420으로 "예" — 실측 교차점과 같다.
- 종단 효과 (`docs/deep-traversal.md`, Cypher 표면):

  | depth | 재작성됨 | `og_vlp` 강제 |
  |---|---|---|
  | 2 | 2.39 ms | 2.08 ms *(둘 다 `og_vlp` — 손익분기 미달)* |
  | 4 | **204.43 ms** | 805.58 ms |
  | 5 | **263.22 ms** | 17,714.73 ms |
  | 8 | **277.39 ms** | 완료되지 않음 |

- 모든 경우가 **양방향으로** 회귀 스위트에 고정되어 있다
  (`engine/tests/sql/05_reachability.sql`): 재작성되는 경우, 재작성되지 않는 경우, 그리고
  시작점으로 되돌아오는 순환 그래프에서 양쪽이 같은 4노드를 반환하되 `count(y)`는 여전히
  6개 트레일을 반환한다는 것까지.

## 결과

**긍정적**
- 최적화가 의미론을 바꾸지 않는다. `WITH` 하나만 있어도 안전 쪽으로 넘어간다.
- 판단 비용이 카탈로그 조회 1회. 계획 시간에 실질적 영향이 없다.
- 얕은 질의가 느려지지 않는다.

**부정적 / 감수한 대가**
- **`WITH`를 쓰는 질의는 아무리 깊어도 재작성되지 않는다.** 이는 보수성의 직접적 비용이며,
  `WITH` 내부를 분석하는 패스가 없기 때문이다.
- 차수를 그래프 전체 평균으로 잡는다 (`:54-56`). 관계 타입별 차수가 크게 다르면 판단이
  거칠어진다 — 주석이 *"this decision only has to be right about an order of magnitude"*
  로 그 거칠음을 의도로 선언한다.
- `WALKS = 512`는 **유도된 값이 아니라 측정에 맞춘 값**이다 (`:36`: *"Fitted to
  measurement rather than derived"*). 다른 하드웨어/형태에서 최적이 아닐 수 있다.
- ANALYZE되지 않은 DB는 깊이 4라는 고정 규칙으로 떨어진다.

## 재검토 조건

- `WITH` 내부를 분석하는 패스가 생기면 게이트 (a)의 가장 큰 손실을 회수할 수 있다.
- 관계 타입별 차수 통계를 저렴하게 얻을 수 있게 되면 게이트 (b)를 타입 단위로 정밀화한다.
- 프론티어 중첩 통계가 생기면 `og_reach_sql`을 3번째 선택지로 편입한다 (ADR-010, ADR-012).
- `WALKS` 임계값은 하드웨어가 크게 바뀌거나 SPI 왕복 비용이 달라지면 재측정 대상이다.

<!-- affects: cypher, performance, correctness -->
<!-- requires-update: docs/99_decisions/ADR-012-visited-set-bfs-rewrite.md -->
