# Tasks: 온톨로지 타입 시스템

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — catalog & single inheritance
- [x] T001 카탈로그 스키마 (type/parent/label/property/role/rule) — `sql/bootstrap.sql`
- [x] T002 nested-set 구간 라벨 할당, GAP=1024 — `catalog/labeling.rs`
- [x] T003 `og_subtypes` / `og_supertypes` / `og_is_subtype` — 범위 비교 1회
- [x] T004 상위 타입 질의 계획에 재귀 노드 0개 (SC-003)

## Phase 1 — multiple inheritance
- [x] T005 다중 부모 → 경로별 라벨 행
- [x] T006 순환 탐지 후 거부 — `labeling.rs::relabel_graph`
- [x] T007 abstract 타입: 저장 테이블 없음, 인스턴스화 거부
- [x] T008 상속 프로퍼티를 자식 테이블로 전파

## Phase 2 — roles
- [x] T009 role 선언 + ordinal(source/target/n-ary)
- [x] T010 엣지 생성 시 role player 타입 강제 — `storage/mod.rs::validate_roles`
- [x] T011 n-ary role player (`og_role_player`)
- [ ] T012 role specialization (하위 관계가 role을 좁히기)

## Phase 3 — constraints
- [x] T013 required → NOT NULL, key → UNIQUE 인덱스 (PostgreSQL이 강제)
- [x] T014 기존 데이터 위반 시 제약 추가 거부
- [x] T015 위반 오류에 타입·role·기대값 포함
- [ ] T016 값 도메인(enum/정규식) CHECK 생성

## Phase 4 — evolution
- [x] T017 선택적 프로퍼티 추가가 기존 행을 재작성하지 않음
- [x] T018 스키마 변경이 트랜잭션 안에서 롤백 가능
- [x] T019 `schema_version` 기록 — 에이전트 캐시 무효화 키
- [x] T020 스키마 변경 시 생성 뷰 자동 무효화
- [ ] T021 계층 중간 삽입 시 국소 재할당 (현재는 전체 재라벨)

## Phase 5 — inference
- [x] T022 관계 특성 선언 (`transitive`/`symmetric`/`reflexive`/`inverse`)
- [ ] T023 질의 시 추론 확장 + 추론/명시 사실 구분
- [ ] T024 추론 깊이·시간 상한

## Phase 6 — introspection
- [x] T025 `og_type_view` / `og_property_view` / `og_role_view`
- [ ] T026 DDL 내보내기 후 재적용 시 카탈로그 100% 재현
