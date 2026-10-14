# Tasks: 스키마 진화

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-09-30

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다
(`engine/tests/sql/08_schema_evolution.sql`, `engine/tests/sql/10_evolution_temporal_coverage.sql`,
`tests/run.sh` 가 실행).
체크되지 않은 항목은 착수하지 않았거나 부분 구현이거나, 동작하지만 테스트가 없는 것이며,
이유는 plan.md 의 Complexity Tracking 에 있다.

## Phase 0 — 변경 레코드
- [x] T001 `og_catalog.schema_change` — 종류, 명세, 단계, 시작/종료 버전, 생성자, 보고
- [x] T002 단계 상태 기계, 순서 위반은 `OG_SCHEMA_PHASE`(55000) (FR-002)
- [x] T003 `og_schema_plan` — 무변경 영향 분석, 위반 표본 id (FR-003)
- [x] T004 `og_schema_changes(graph)` — 최신순 목록
- [x] T005 버전마다 카탈로그 스냅샷, `og_schema_snapshot(graph, version)` (FR-009)
- [x] T006 `og_schema_snapshot(graph, at => timestamptz)` (10: 그래프 생성 뒤 시각은 스냅샷, 이전 시각은 NULL)
- [ ] T007 `og_create_index`, `og_map_table`, `og_materialize_mapping` 이 스키마 버전을 올림 (FR-020)
      — 구현. `og_create_index` 는 10 이 단언한다. `og_map_table`·`og_materialize_mapping` 에 대한
      전용 단언은 없다
- [x] T008 `og_add_property(required)` 가 관계 인스턴스도 센다 (002 결함 수정) (10)

## Phase 1 — 카탈로그 수준 변경
- [x] T009 `rename_property` — 확장 중 두 이름 모두 읽기/쓰기, 하위 타입 전파, 축소 후 옛 이름 제거
- [ ] T010 `rename_property` 가 `write_grant`·`compat_index` 의 속성 이름을 따라감 — 구현, 테스트 없음
- [x] T011 `rename_type` — 새 이름으로 같은 노드 반환
- [x] T012 `drop_property` — 컬럼 제거 (`og_schema_apply`)
- [x] T013 상속된 속성은 선언 타입에서만 변경 (FR-019)
- [x] T014 `add_parent` — 기존 행 `xmin` 불변, 상위 타입 질의에 포함
- [x] T015 `add_parent` 순환 거부
- [x] T016 `remove_parent`
- [ ] T017 `add_property` 를 파이프라인으로 — 구현(`og_add_property` 위임), 08 에서 직접 호출 없음

## Phase 2 — 표현 변경
- [x] T018 `alter_property_type` 계획 — 캐스트 불가 값 수와 표본
- [x] T019 확장 후 새 쓰기가 새 타입으로 캐스트되지 않으면 거부, 읽기는 옛 컬럼
- [x] T020 배치 마이그레이션, 재개 가능한 id 커서 (batch 2 × 5행 → 3회 이상 호출 단언)
- [x] T021 검증이 위반 행 id 를 보고, 축소 거부
- [x] T022 축소 — 같은 이름의 새 타입, 수치 비교, 인덱스 재생성, 하위 타입 카탈로그 갱신
- [x] T023 `og_schema_abort` — 그림자 제거, 이중 쓰기 중단
- [x] T024 이력 테이블(`h_<tid>`) 동반 마이그레이션 (10: temporal 타입의 이력 컬럼이 새 타입으로 바뀜)
- [x] T025 전환 중 속성 단위 쓰기 권한 유지 — 그림자는 트리거가 채우므로 별도 권한 불필요 (10 이 단언)
- [x] T026 원시 SQL 쓰기의 이중 쓰기 — 트리거 `og__shadow_write` (08: 전환 중 원시 SQL 의 캐스트 불가 값
      거부(22P02), 마이그레이션된 행의 원시 SQL 갱신이 축소 후에도 남음)

## Phase 3 — 제약
- [x] T027 `set_required` — 계획이 빈 행을 셈, 확장 후 새 쓰기 거부(23514), 기본값 채우기, `NOT NULL`
- [x] T028 `add_unique` — 계층에 걸친 중복 보고, 수정 후 축소, 같은 테이블 중복 거부(23505)
- [x] T029 `alter_role` 위반 시 `og_schema_apply` 가 `OG_SCHEMA_VIOLATION` 으로 실패하고 역할 불변
- [x] T030 `alter_role` 성공 경로(축소) (10: `card_max` 갱신)
- [ ] T031 `add_unique`/`alter_role` 의 확장~축소 사이 새 쓰기 강제 — **미구현**
- [ ] T032 계층 전체 유니크 강제 — **미구현**

## Phase 4 — not yet
- [ ] T033 타입 분리/병합 (노드 재식별 + 인접 재작성)
- [ ] T034 TypeQL `redefine` / `undefine` 을 이 파이프라인으로
- [ ] T035 데이터 크기 대비 비용 측정 — `docs/schema-evolution-benchmark.md`
