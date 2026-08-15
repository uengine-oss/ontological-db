# Tasks: 네이티브 그래프 저장 엔진

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — identifiers & type tables
- [x] T001 식별자 비트 인코딩 `[shard:9][type:18][local:36]` — `engine/src/id.rs`
- [x] T002 오버플로 시 조용한 절단 대신 오류 — `id.rs::make_id`
- [x] T003 타입별 저장 테이블 DDL 생성기 — `catalog/types.rs::og_create_type`
- [x] T004 선언 프로퍼티 → 실제 컬럼, 미선언 → `__ext` jsonb — `storage/mod.rs::plan_props`
- [x] T005 타입별 local id 할당기 — `storage/mod.rs::alloc_id`
- [x] T006 노드/엣지 CRUD, 값은 전부 바인딩 파라미터 경유

## Phase 1 — adjacency segments
- [x] T007 `og_adj` 스키마, CHUNK=256, `STORAGE MAIN` — `sql/bootstrap.sql`
- [x] T008 append: 꼬리 청크 갱신, 가득 차면 새 청크 — `storage/adjacency.rs`
- [x] T009 remove: 두 배열 동일 인덱스 splice + 빈 세그먼트 회수
- [x] T010 `og_expand` / `og_expand_batch` — inlinable SQL (플래너 통과) — `sql/access.sql`
- [x] T011 방향·타입별 분할로 프루닝 (FR-003)
- [x] T012 엣지 생성 시 양방향 인접을 같은 트랜잭션에서 갱신 (FR-012)

## Phase 2 — statistics & bulk
- [x] T013 `og_degree` / `og_degree_all` — 비용 추정 입력
- [x] T014 `og_graph_stats` — 타입별 개수, 패킹 비율, 슈퍼노드 수
- [x] T015 `og_degree_distribution` — 차수 히스토그램
- [x] T016 벌크 적재 경로 (bench 하네스가 사용)

## Phase 3 — operations
- [x] T017 `og_reorganize` — 온라인 세그먼트 재패킹
- [x] T018 `og_check_integrity` — 인접/레지스트리/타입 테이블 교차 검증
- [x] T019 vacuum·pg_dump 정합 (일반 힙 릴레이션이므로 자동)

## Phase 4 — deferred to v2
- [ ] T020 Table Access Method 전환 (plan.md Complexity Tracking 참조)
- [ ] T021 슈퍼노드 자동 청크 임계치 튜닝
