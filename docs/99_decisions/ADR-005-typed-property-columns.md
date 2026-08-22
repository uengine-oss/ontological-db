# ADR-005: 프로퍼티를 타입 카탈로그가 생성한 실제 컬럼으로 저장한다 (미선언은 `__ext` jsonb)

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/001-graph-storage-engine/plan.md` 기준일) |
| 영향 범위 | storage, catalog, cypher, vector, security |
| 근거 | `.specify/memory/constitution.md` 원칙 III·"금지 사항", `specs/001-graph-storage-engine/plan.md` "타입별 프로퍼티 테이블", `engine/src/vector/mod.rs:1-13`, `README.md` 비교표 |

> **이 문서가 답하는 질문**
> - 왜 프로퍼티를 `agtype`/JSONB 한 컬럼에 담지 않는가?
> - 선언되지 않은 프로퍼티는 어디로 가는가?

## 배경

Apache AGE는 프로퍼티를 `agtype`(JSON) 컬럼 하나에 담는다. 그 결과 **모든 프로퍼티 읽기가
JSON 파싱**이고, 더 중요하게는 **옵티마이저가 컬럼 통계를 갖지 못한다.** README가 이를
구조적 문제로 지목한다:
> AGE does rewrite `cypher()` into a query tree, but `agtype` leaves it without column
> statistics or ordinary indexes, so join order is chosen from defaults rather than
> from the data.

헌법은 이 구조를 **금지된 안티패턴**으로 명시한다: *"노드/엣지를 각각 하나의 일반 힙
테이블에 담고 프로퍼티를 JSONB/agtype으로 저장하는 구조."*

## 고려한 선택지

1. **`agtype` / JSONB 단일 컬럼** — 스키마리스로 편하다. 헌법이 금지. 통계·인덱스·타입
   보존을 모두 잃는다.
2. **고정폭 슬롯 자체 직렬화** — 헌법 원칙 III가 언급한 "타입 기반 슬롯 저장". TableAM
   없이는 힙 튜플 안에서 이를 직접 구현할 수 없다 (ADR-002).
3. **타입 카탈로그가 DDL로 생성하는 실제 컬럼 + 잔여분 `__ext` jsonb**

## 결정

**3안.** 타입마다 저장 테이블을 만들고, 선언된 프로퍼티는 **실제 타입 컬럼**이 된다.

```sql
og_node_<type_id> ( id int8 PRIMARY KEY, <declared props as real columns...>, __ext jsonb )
og_edge_<type_id> ( id int8 PRIMARY KEY, src int8, dst int8, <props...>, __ext jsonb )
```

선언되지 않은 프로퍼티는 버리지 않고 `__ext` jsonb로 흘려보낸다
(`engine/src/storage/mod.rs:42-46`). 컬럼 추가는 PG11+ 에서 테이블 재작성 없이 즉시
완료된다 (`specs/001-.../plan.md`).

## 근거

- 컴파일된 SQL이 이 결정의 증거다. README "Look at the SQL" 예시에서
  `p_born > 1960`은 **인덱싱 가능한 실컬럼 술어**이며, 플래너가 통계를 갖는다.
- 벡터가 "관계에도 1급"인 이유가 정확히 이 결정이다
  (`engine/src/vector/mod.rs:3-8`):
  > There is no separate embedding store. An embedding is a `vector(N)` property,
  > which spec 002 turns into a real column on the type table … That single decision
  > is what makes **relationship embeddings first class**.
- 사후 필터링이 원천적으로 불가능해지는 이유도 같다 (`engine/src/vector/mod.rs:9-13`):
  라벨이 이미 구체 테이블로 해소되어 있으므로 *"the graph predicate and the ANN index
  live on the same relation. There is nowhere for a post-filter to hide."*
- 스칼라가 아닌 값(배열/객체)은 의도적으로 `__ext`에 남긴다
  (`engine/src/storage/mod.rs:49-52`): 유일하게 중요한 배열 프로퍼티인 `embedding`은
  `og_add_embedding`이 `vector(N)`으로 선언하며, 먼저 jsonb로 선언해버리면 그 길이 막힌다.

## 결과

**긍정적**
- 통계·인덱스·MVCC·RLS를 공짜로 얻는다. HNSW 인덱스가 엣지 프로퍼티에도 그대로 붙는다.
- 프로퍼티 타입이 왕복에서 보존된다.

**부정적 / 감수한 대가**
- **타입 하나 = 테이블 하나.** 타입이 많은 온톨로지는 릴레이션 수가 많아지고,
  `pg_class`/`pg_attribute` 부하와 계획 시간이 늘어난다.
- 라벨 해소가 여러 하위 타입 테이블의 `UNION` 뷰가 된다 (`engine/src/cypher/views.rs`).
- `__ext`에 남은 프로퍼티는 인덱스도 통계도 없다 — 이 비대칭이 ADR-006(쓰기 시점 승격)을
  낳았다.
- 스키마 변경(프로퍼티 추가)이 DDL이므로 카탈로그 락을 잡는다.

## 재검토 조건

- 타입 수가 수천 개 규모로 늘어나 릴레이션 수가 계획 시간·카탈로그 캐시를 지배할 때 —
  희소 타입을 공용 테이블 + 태그 컬럼으로 접는 하이브리드를 재평가한다.
- `__ext` 잔여 비율이 유의미하게 높아지면(측정: `og_schema`/`og_graph_stats`),
  승격 규칙(ADR-006)의 범위를 스칼라 밖으로 넓힐지 재검토한다.

<!-- affects: storage, catalog, cypher, vector, security -->
<!-- requires-update: docs/99_decisions/ADR-006-write-time-property-promotion.md -->
