# ADR-014: 백엔드-로컬 CSR(`og_csr_build`)을 자동으로 적용하지 않는다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-17 (커밋 `f9e64fd`) |
| 영향 범위 | storage, cypher, security, ops |
| 근거 | `engine/src/storage/traverse.rs:19-23`, `:205-210`, `docs/deep-traversal.md` "What the CSR is, and what it costs", `README.md` "Past three hops" |

> **이 문서가 답하는 질문**
> - 15배 빠른 경로가 있는데 왜 기본값이 아닌가?
> - `og_csr_build`는 언제 써야 하는가?

## 배경

ADR-012가 힙 안에서 691배를 벌고 난 뒤, 남은 질문은 "힙을 벗어나면 얼마나 더 빨라지는가"
였다. `docs/comparison.md`가 pgGraph에게 깊은 순회를 양보하며 *"a cached in-memory mirror
is the obvious thing to consider, and it is not built"* 라고 적었고, 그것을 실제로 만들어
측정한 결과가 `og_csr_build` / `og_csr_reach` / `og_csr_hops` / `og_csr_stats` /
`og_csr_drop` 이다.

결과: **또 한 자릿수 배수**. dense 6홉에서 `og_reach` 71.42ms 대 `og_csr_reach` 4.86ms.

## 고려한 선택지

1. **CSR을 기본 경로로** — 가장 빠른 숫자를 기본값으로 삼는다.
2. **CSR을 만들지 않음** — 힙 밖 캐시는 헌법 원칙 IX("캐시는 진실의 원천이 될 수 없다")를
   자극한다.
3. **CSR을 만들고, 측정하고, 문서화하되 — 컴파일러가 자동으로 고르지 않는다**

## 결정

**3안.** Cypher 컴파일러는 `og_reach`로만 라우팅한다. CSR은 명시적으로 `og_csr_build()`를
호출한 세션에서만 쓰인다.

`docs/deep-traversal.md`가 결정을 한 문장으로 적는다:
> Which is why the Cypher compiler routes to `og_reach` and not to the CSR. The CSR is
> exposed, measured and documented; it is not silently substituted for a query whose
> caller is entitled to MVCC and RLS.

## 근거

CSR이 포기하는 것은 정확히 힙을 떠나면 포기하는 것들이다
(`engine/src/storage/traverse.rs:19-23`):
> Compile the topology once into a backend-local CSR of dense `u32` indices and walk it
> with no SPI, no heap and no planner in the loop. Faster, and it gives up exactly what
> leaving the heap gives up: the snapshot is frozen at build time and RLS is never
> consulted.

`docs/deep-traversal.md`가 대가를 항목별로 기록한다.

| 대가 | 내용 |
|---|---|
| 백엔드 단위 비용 | dense 119ms / 8.4MiB, sparse 229ms / 9.2MiB. **연결마다** 지불 |
| 스냅샷 동결 | 빌드 이후 커밋된 엣지는 재빌드 전까지 보이지 않는다. 트리거 캡처 없음 |
| RLS 미적용 | 호출자가 읽을 수 없는 행을 지나는 경로가 결과에 나타난다 |
| 병렬 불가 | `PARALLEL RESTRICTED` — 계획이 리더에서 실행된다 |

- 메모리 상주 이유도 명시적이다 (`engine/src/storage/traverse.rs:206-209`):
  > One compiled graph per backend. Rust-heap allocated on purpose: a PostgreSQL memory
  > context would free it at end of transaction, and the whole point is that the next
  > statement finds it already built.
- 헌법 원칙 IX가 캐시를 허용하되 *"캐시는 진실의 원천이 될 수 없다"* 고 규정한다.
  자동 적용은 이 조항을 조용히 위반한다.
- README도 이 상태를 표에 그대로 적는다:
  `compiled backend-local CSR (og_csr_build, not automatic) | 4.9 ms`.

## 결과

**긍정적**
- 기본 경로가 MVCC·RLS·같은 트랜잭션의 미커밋 쓰기를 모두 유지한다. 보안 속성이 최적화에
  의해 조용히 사라지지 않는다.
- pgGraph 아키텍처의 실제 가치가 **측정값으로** 남았다 — 논쟁이 아니라 숫자로. 같은 문서가
  pgGraph 자신의 CSR 워크(42ms)와 우리 CSR(4.86ms)을 나란히 놓는다.

**부정적 / 감수한 대가**
- 대부분의 사용자는 15배를 얻지 못한다. 명시적으로 호출해야 한다.
- 세션마다 다시 빌드해야 하므로 **연결 풀링이 없는 배포에서는 사실상 쓸 수 없다.**
  pgGraph의 공개된 cold 컬럼(질의 종류와 무관하게 2.8~3.4초)이 같은 효과다.
- 두 개의 순회 구현을 유지해야 한다.

## 재검토 조건

- **트리거 기반 변경 캡처**가 구현되어 CSR이 스냅샷 동결을 벗어나면, 그때 자동 적용을
  재평가한다. `docs/deep-traversal.md`가 이를 *"the obvious next thing if this path is
  kept"* 로 명시했다.
- RLS를 CSR 경로에서 강제할 방법이 생기면(예: 정책을 빌드 시점에 반영), 두 번째 장벽이
  사라진다.
- 두 장벽이 모두 해소되기 전에는 **자동 적용을 검토하지 않는다.** 성능 이득만으로는
  헌법 원칙 IX와 spec 005의 RLS 보장을 넘어설 수 없다.

<!-- affects: storage, cypher, security, ops -->
<!-- requires-update: docs/99_decisions/ADR-012-visited-set-bfs-rewrite.md -->
