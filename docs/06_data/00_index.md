# 06_data — 데이터 모델

> **이 문서가 답하는 질문**
> - 이 카테고리에는 무엇이 들어 있고, 어떤 순서로 읽어야 하는가?
> - "물리 스키마가 곧 제품"이라는 말은 이 프로젝트에서 정확히 무엇을 뜻하는가?
> - 데이터 모델에 대해 무엇을 사실로 믿어도 되고, 무엇이 아직 미확인인가?

---

## 이 카테고리의 역할

Ontological은 PostgreSQL **확장(extension)**이다. 별도 스토리지 엔진도, 커스텀
Table Access Method도, 포크된 서버도 없다(`engine/sql/bootstrap.sql:8-10`).
그래서 이 제품의 성능·정합성·운영 특성은 **전부 물리 스키마의 성질**로 환원된다.

- 순회가 빠른 이유는 알고리즘이 아니라 `og_data.og_adj`가 이웃을 배열로 묶어
  힙 튜플 하나에 담기 때문이다(`engine/sql/bootstrap.sql:186-214`).
- 서브타입 판정이 상수시간인 이유는 재귀를 안 써서가 아니라
  `og_catalog.type_label`이 구간 라벨을 들고 있고 그 위에 범위 인덱스가 있기
  때문이다(`engine/sql/bootstrap.sql:68-80`).
- 프로퍼티에 통계·인덱스·MVCC·RLS가 붙는 이유는 그것이 jsonb 블롭이 아니라
  **실제 컬럼**이기 때문이다(`engine/src/catalog/types.rs:411-442`).

따라서 이 카테고리는 "테이블 목록"이 아니라 **각 구조가 무엇을 의미하고,
언제 만들어지고, 언제 갱신되고, 언제 사라지는가**를 기술한다.

---

## 문서 목록

| 문서 | 답하는 질문 | 주 독자 |
|---|---|---|
| [`01_physical_schema.md`](01_physical_schema.md) | `og_catalog` / `og_data`에 어떤 테이블·컬럼·제약·인덱스가 있고 각각 무엇을 의미하는가? | 전원 |
| [`02_identifier_encoding.md`](02_identifier_encoding.md) | 64bit id는 어떻게 쪼개져 있고, 왜 type_id를 id에 박았으며, 한계는 무엇인가? | 백엔드, 운영 |
| [`03_adjacency_model.md`](03_adjacency_model.md) | CSR 세그먼트는 물리적으로 어떻게 생겼고 어떻게 커지고 줄어드는가? | 백엔드, 성능 |
| [`04_type_catalog_model.md`](04_type_catalog_model.md) | 타입 DAG와 구간 라벨의 수학적 성질, 재라벨링 비용은? | 백엔드, 온톨로지 설계자 |
| [`05_property_model.md`](05_property_model.md) | 타입 컬럼과 `__ext` jsonb의 경계는 어디이며 누가 언제 옮기는가? | 백엔드, 앱 개발자 |
| [`06_role_and_relation_model.md`](06_role_and_relation_model.md) | role / n-항 관계 / TypeQL reification은 어떤 행으로 저장되는가? | 온톨로지 설계자 |
| [`07_vector_data_model.md`](07_vector_data_model.md) | 임베딩은 어디에 저장되고 HNSW 인덱스는 어떤 파라미터로 만들어지는가? | 검색/RAG |
| [`08_data_lifecycle.md`](08_data_lifecycle.md) | 생성→사용→폐기 전 구간에서 무엇이 남고 무엇이 사라지는가? | 운영, DBA |
| [`09_query_access_paths.md`](09_query_access_paths.md) | 어떤 질의 형태가 실제로 어떤 인덱스를 타는가? | 백엔드, 성능 |
| [`10_improvements_data.md`](10_improvements_data.md) | 데이터 모델·인덱스·성능의 개선 포인트는 무엇인가? | 전원 |

---

## 읽는 순서 (역할별)

- **처음 오는 사람**: `01` → `03` → `05`
- **성능 문제를 쫓는 사람**: `09` → `03` → `10`
- **운영/DBA**: `08` → `01` → `10`
- **LLM 에이전트**: `01`(스키마 정본) → `09`(접근 경로) → `10`(알려진 결함).
  기능 지원 여부를 답하기 전에는 [`../00_overview/05_spec_status.md`](../00_overview/05_spec_status.md)를
  먼저 볼 것.

---

## 이 카테고리의 정본(source of truth)

| 대상 | 정본 파일 |
|---|---|
| 부트스트랩 스키마 전체 | `engine/sql/bootstrap.sql` (448줄) |
| 공개 접근 경로 함수/뷰 | `engine/sql/access.sql` (338줄) |
| 런타임 생성 테이블 DDL | `engine/src/catalog/types.rs`, `engine/src/typeql/schema.rs` |
| 쓰기 경로 | `engine/src/storage/mod.rs`, `engine/src/storage/adjacency.rs` |
| 성능 근거 수치 | `docs/benchmark.md`, `docs/deep-traversal.md` |

**이 문서들과 위 파일이 어긋나면 파일이 맞다.** 문서 갱신 누락을 발견하면
`10_improvements_data.md`가 아니라 해당 문서를 고칠 것.

---

## 금지 / 필수 (전 문서 공통)

**금지**
- `og_data.*` 테이블에 애플리케이션이 직접 `INSERT` / `UPDATE` / `DELETE` 하는 것.
  단, 벌크 로드는 예외이며 그 조건은 [`08_data_lifecycle.md`](08_data_lifecycle.md)에 있다.
- `og_data.og_adj`의 `nbr` / `eid` 배열을 SQL로 직접 편집하는 것.
  두 배열의 정렬(alignment)과 `n` 컬럼이 동시에 맞아야 하며, 이를 깨면
  `og_check_integrity()`의 `segment_length_mismatch`가 뜬다
  (`engine/src/storage/stats.rs:225-241`).
- `og_catalog.type_label`을 손으로 고치는 것. `og_relabel(graph_id)`만이 정당한 경로다
  (`engine/src/catalog/labeling.rs:247-249`).

**필수**
- 벌크 로드 직후 `ANALYZE`. 확장 코드 어디에도 `ANALYZE` 호출이 없다
  (근거: `engine/src/` 전체에서 `ANALYZE` 매치는 `engine/src/cypher/mod.rs:682`의
  `EXPLAIN` 옵션 문자열 하나뿐).
- 프로퍼티를 인덱싱할 계획이 있다면 **쓰기 전에** `og_add_property()`로 선언할 것.
  나중에 선언해도 `__ext`에서 컬럼으로 옮겨주지만(`engine/src/catalog/types.rs:561-570`),
  그 UPDATE는 해당 타입의 모든 행을 재작성한다.

---

<!-- affects: data, backend, ops -->
