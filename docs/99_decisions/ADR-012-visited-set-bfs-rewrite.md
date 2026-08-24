# ADR-012: 가변 길이 경로를 트레일 열거에서 방문집합 BFS로 재작성한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-17 (커밋 `f9e64fd` "Deep traversal: compile reachability instead of enumerating trails") |
| 영향 범위 | cypher, storage, performance |
| 근거 | `engine/src/storage/traverse.rs:1-25`, `docs/deep-traversal.md`, `README.md` "Past three hops", `engine/sql/access.sql:138-186` |

> **이 문서가 답하는 질문**
> - 6홉 질의가 왜 49초였고 왜 71ms가 되었는가?
> - 저장 구조를 바꾸지 않고 어떻게 691배가 나오는가?

## 배경

Cypher의 가변 길이 매치는 **경로 1개당 행 1개**를 낳는다. `og_vlp`
(`engine/sql/access.sql:138-`)는 그 의미론을 충실히 구현한다 — 엣지 id 배열을 들고 다니고,
반복 엣지를 거부하며(trail 의미론), 걷기마다 한 행을 반환한다.

문제는 애플리케이션이 실제로 묻는 질문이 다르다는 것이다.
`RETURN count(b)`는 **걷기 수**를 세고, `RETURN count(DISTINCT b)`는 **노드 수**를 센다.
두 번째를 첫 번째로 답하면 비용이 `Σ degreeⁱ` 행인데 답은 `|V|`로 유계다.

평균 차수 20 그래프에서 홉당 20배. **6홉이 49초 걸려 50,000 노드를 보고했다**
(`README.md` "Past three hops").

## 고려한 선택지

1. **저장 구조 교체(인메모리 미러/CSR로 이관)** — `docs/comparison.md`가 원래 제시한
   방향. 실제로 만들어 측정한 결과 **15배**를 벌었다 (ADR-014).
2. **`og_reach_sql` — 경로 배열 없이 `UNION` 재귀 CTE** — 새 코드가 필요 없다.
   그러나 `(node, depth)`가 서로 다른 행이므로 순환 그래프에서 `O(k·|V|)`로 퇴화한다.
3. **방문집합 BFS로 컴파일** — 질문이 경로를 관측할 수 없으면 경로를 만들지 않는다.

## 결정

**3안을 주 경로로 채택한다.** 컴파일러가 조건을 만족하는 가변 길이 홉을 `og_vlp` 대신
`og_reach`로 내린다 (`engine/src/cypher/compile.rs:865-870`).

`og_reach`(`engine/src/storage/traverse.rs:81`)는 레벨 동기 BFS이며, 인접 세그먼트를 SPI로
읽는다 — **같은 힙 튜플, 같은 가시성 규칙**이다.

## 근거

- 모듈 주석이 결정의 논리 전체를 담는다 (`engine/src/storage/traverse.rs:12-14`):
  > Reachability does not need paths. A frontier and a visited set touch every node at
  > most once, so the work is bounded by `|V| + |E|` however deep the question goes.
- 저장 구조를 바꾸지 않았다는 점이 이 결정의 핵심이다 (`docs/deep-traversal.md` 요약):
  > **Most of the cost was not the heap.** … Asking the same question as reachability,
  > with a visited set, is bounded by `|V|+|E|` and answers in **71 ms in the heap**
  > with MVCC and RLS fully intact. That is a **691× improvement with nothing lifted
  > out of PostgreSQL.**
- 측정값 (`docs/deep-traversal.md`, dense 50,000노드/999,784엣지, 평균 차수 20):

  | depth | `og_vlp` | `og_reach_sql` | `og_reach` | `og_csr_reach` |
  |---|---|---|---|---|
  | 4 | 106.72 | 49.24 | 23.62 | 3.68 |
  | 6 | 49,333.99 | 426.45 | **71.42** | 4.86 |
  | 20 | — | 3,659.57 | **69.43** | 4.88 |

- 곡선의 형태가 결정을 정당화한다: *"`og_vlp`'s curve is the degree, exactly."* —
  느려지는 원인이 힙도 MVCC도 플래너도 아니라 **행 수**임을 보여준다.
- Cypher 표면 기준 종단 효과: 8홉이 *"does not finish"* 에서 277.39ms로.

## 결과

**긍정적**
- 깊은 순회가 저장 구조 변경 없이 성립한다. MVCC, RLS, 같은 트랜잭션의 미커밋 쓰기가
  모두 유지된다 (`engine/src/storage/traverse.rs:17-18`).
- 깊이가 그래프를 덮은 뒤에는 지연이 깊이와 무관해진다(평탄해진다).

**부정적 / 감수한 대가**
- `og_reach`는 Rust 집합 반환 함수라 **인라인되지 않고 레벨마다 SPI 설정 비용을 낸다**
  (ADR-010). 이것이 ADR-013의 손익분기 규칙을 필요하게 만들었다.
- `PARALLEL RESTRICTED`이므로 이 함수를 포함한 계획은 리더에서 실행된다
  (`docs/deep-traversal.md`).
- **프론티어가 얇고 깊이가 큰 형태에서는 손해다.** 체인 100,000홉에서 `og_reach` 1,016ms
  대 `og_reach_sql` 154ms — 재귀 CTE에 6.6배 진다. 이는 문서가 스스로 기록한 한계다.
- 초기 구현은 루프 **안에서** SPI 연결을 열고 질의를 재계획했다. 그것을 밖으로 들어내
  100,000홉이 1,196ms → 1,016ms가 되었고, 나머지는 SPI 경계를 넘는 비용의 바닥이다.

## 재검토 조건

- 프론티어 중첩 여부를 저렴하게 알려주는 통계가 생기면, `og_reach_sql`을 포함한 3지 선택을
  컴파일러에 넣는다 (`docs/deep-traversal.md`가 명시적으로 남긴 과제).
- SPI 레벨 왕복 비용이 제거 가능해지면(예: 접근 경로가 인라인 가능한 형태로 재작성되면)
  얇고 깊은 그래프에서의 열세가 해소되는지 재측정한다.
- Neo4j가 앞서는 영역 — 프론티어가 얇고 깊이가 큰 형태(격자 500홉 61ms 대 146ms, 체인
  10,000홉 10.9ms 대 91ms) — 이 제품 목표가 되면 이 결정만으로는 부족하다.

<!-- affects: cypher, storage, performance -->
<!-- requires-update: docs/99_decisions/ADR-013-conservative-bfs-rewrite.md, docs/99_decisions/ADR-014-csr-not-automatic.md -->
