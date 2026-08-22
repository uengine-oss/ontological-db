# ADR-004: 인접 정보를 CSR형 세그먼트(힙 튜플당 이웃 ≤256개)로 저장한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/001-graph-storage-engine/plan.md` 기준일) |
| 영향 범위 | storage, cypher, typeql, vector |
| 근거 | `engine/src/storage/adjacency.rs:1-15`, `engine/sql/bootstrap.sql:197-213`, `engine/sql/access.sql:12-23`, `.specify/memory/constitution.md` 원칙 III, `README.md` 비교표 |

> **이 문서가 답하는 질문**
> - 왜 엣지 1개 = 행 1개가 아닌가?
> - 256이라는 숫자는 어디서 나왔는가?

## 배경

Apache AGE는 엣지를 독립된 힙 행으로 저장하고 끝점 컬럼의 B-tree로 이웃에 도달한다.
결과적으로 **한 홉이 `degree`회 인덱스 프로브 + `degree`회 랜덤 힙 페치**가 된다
(`engine/src/storage/adjacency.rs:7-9`). 헌법 원칙 III는 이웃 접근이 "인덱스 조회가 아닌
순차 메모리 접근"이어야 한다고 요구하며, 측정 기준으로 *"1-hop 이웃 확장 비용은 Apache AGE
대비 최소 5배 이상 개선"* 을 건다.

## 고려한 선택지

1. **엣지당 1행 + 끝점 B-tree (AGE 방식)** — 헌법이 금지한 안티패턴. 성능 목표 미달.
2. **GIN/GiST 인덱스** — `plan.md` Complexity Tracking이 기각: *"순차성 없음"*.
3. **인접 리스트를 배열 컬럼으로 묶은 세그먼트(CSR 계열)** — 한 노드의 한 타입·한 방향
   이웃을 정렬된 `int8[]` 두 개(`nbr`, `eid`)로 저장.

## 결정

**3안.** `og_data.og_adj`를 다음 형태로 둔다 (`engine/sql/bootstrap.sql:197-`).

```sql
og_adj (
  src   int8, etype int4, dir "char", seq int4, n int4,
  nbr   int8[],   -- 이웃 노드 ID
  eid   int8[],   -- 대응 엣지 ID
  PRIMARY KEY (src, etype, dir, seq)
)
```

- 세그먼트당 이웃 수 상한은 `CHUNK = 256` (`engine/src/storage/adjacency.rs:15`).
- `dir`('o'/'i')와 `etype`을 키에 분리해 방향별·타입별 프루닝을 인덱스 범위로 얻는다.
- 배열이 TOAST로 빠져나가지 않도록 `STORAGE MAIN`을 명시한다
  (`engine/sql/bootstrap.sql:210-211`).

## 근거

- 청크 크기의 산정 근거가 주석에 있다 (`engine/src/storage/adjacency.rs:13-14`):
  *"256 * 8B * 2 arrays = 4 KB, comfortably inside one 8 KB heap page, which is what
  keeps the read sequential."*
- 접근 함수도 같은 주장을 반복한다 (`engine/sql/access.sql:12`, `:25-26`):
  *"One heap tuple per 256 neighbours."* / *"Sequential array read, not degree index
  probes."*
- README 비교표가 이 결정을 AGE와의 첫 번째 구조적 차이로 명시한다:
  `adjacency | one row per edge + B-tree | CSR-style segments: ≤256 neighbours per heap tuple`

**계획서와 구현의 불일치 (사실).**
`specs/001-graph-storage-engine/plan.md`는 `CHUNK_SIZE = 1024`로 적혀 있으나
**구현은 256이다** (`engine/src/storage/adjacency.rs:15`, `engine/sql/bootstrap.sql:214`).
계획서의 1024는 "1024 × 8바이트 = 8KB 페이지 경계"라는 계산이었고, 구현은 배열이 **두 개**
(`nbr`, `eid`)라는 점을 반영해 256 × 8B × 2 = 4KB로 낮췄다. 4KB는 컬럼당 TOAST 임계
(2KB × 4)를 넘지 않아 `STORAGE MAIN`으로 인라인 유지가 가능하다
(`engine/sql/bootstrap.sql:208-211`). **현재 유효한 값은 256이다.**

**단, 이 결정의 효과는 README가 스스로 정정한 범위 안에서만 주장된다.**
`README.md` "Why this exists"는 다음을 덧붙인다:
> Measurement puts almost all of the observed cost somewhere narrower than either
> bullet: a single indexed hop through AGE reads about as many pages as we do, while
> its variable-length path operator rescans at every depth.

즉 **1홉 저장 구조만 놓고 보면 인덱스를 제대로 갖춘 AGE와 대등**하며, 실제 격차는 AGE의
`*1..n` 연산자에서 나온다 (`docs/benchmark.md`). 이 ADR은 그 사실을 감추지 않는다.

## 결과

**긍정적**
- 한 노드의 한 타입 이웃 확장이 배열 1개 = 페이지 1~2개 순차 읽기가 된다.
- `og_expand`가 `LANGUAGE sql`로 인라인되므로 플래너가 이 스캔을 직접 본다 (ADR-010).
- 정방향/역방향을 모두 저장하므로 양방향 최단경로가 추가 I/O 없이 성립한다
  (`engine/src/storage/traverse.rs:180-186`).

**부정적 / 감수한 대가**
- **엣지 1개 추가가 배열 재작성 = 새 힙 튜플 버전을 만든다.** `adjacency.rs:19-45`의
  `append`는 꼬리 세그먼트를 `UPDATE`한다. 쓰기 증폭은 이 설계의 구조적 비용이다.
- 엣지 삭제는 배열 스플라이스이며(`adjacency.rs:48-63`), 세그먼트가 비어도 즉시 회수되지
  않는다 (`og_reorganize`가 별도로 존재하는 이유).
- `og_adj`는 이웃 id만 담으므로 **RLS 정책을 걸 수 없다.** 그래서 컴파일러가 항상 대상
  노드 뷰와 조인해 RLS를 통과시킨다 (`specs/005-postgres-supabase-interop/plan.md`
  "RLS 적용 지점"). 이 조인은 선택이 아니라 안전 요구사항이다.
- 논리 복제에서 배열 기반 인접 구조의 표현이 자명하지 않아 v1은 물리 복제로 대체했다
  (ADR-024 관련: `specs/005-.../plan.md` Complexity Tracking).

## 재검토 조건

- 차수 분포가 극단적으로 치우친 그래프(슈퍼노드 다수)에서 `append`의 쓰기 증폭이 적재
  처리량을 지배할 때 — `og_degree_distribution`/`og_graph_stats`가 그 신호원이다.
- PostgreSQL의 배열 갱신이 부분 갱신(in-place append)을 지원하게 되면 청크 크기와 갱신
  전략을 재산정한다.
- TableAM으로 전환하면(ADR-002 재검토) 이 구조는 힙 튜플이 아니라 전용 페이지 레이아웃으로
  옮겨간다.

<!-- affects: storage, cypher, security, ops -->
<!-- requires-update: docs/99_decisions/ADR-002-no-table-access-method.md -->
