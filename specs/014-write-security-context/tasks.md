# Tasks: 쓰기 시점 보안 컨텍스트

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-09-30

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다
(`engine/tests/sql/07_write_security.sql` — 비소유자 역할로 실행,
`engine/tests/sql/10_evolution_temporal_coverage.sql` 의 014 구간, `tests/bolt-context/run.py`
— 공식 neo4j 드라이버). 체크되지 않은 항목은 착수하지 않았거나, 구현되었지만 테스트가 없는
것이다.

## Phase 0 — 비소유자 역할로 드러난 선행 결함
- [x] T001 `og_data` CREATE 권한 없는 읽기 역할이 아직 없는 생성 뷰를 통해 읽기 (`og__ensure_view`)
- [x] T002 스키마 권한 없는 쓰기 역할의 미선언 속성은 `__ext` 로 (ADR-025 서술대로)
- [x] T003 `ensure_alias_view` 이중 인용 — `og_grant` 이후 `og_add_property` 실패
- [x] T004 쓰기 역할이 `og_audit`/`og_history`/`h_*` 를 수정·삭제할 수 있던 것

## Phase 1 — 쓰기 권한
- [x] T005 `og_grant_write` — 연산·속성 단위, 하위 타입 전파
- [x] T006 `OG_WRITE_DENIED` — 속성 / 미선언 속성 / 삭제 / 다른 타입, SQLSTATE 42501
- [x] T007 원시 SQL `UPDATE`/`DELETE` 도 거부
- [x] T008 레지스트리 RLS — 쓸 수 없는 타입의 `og_node` 행 INSERT 거부
- [x] T009 `og_revoke_write(…, ops)` — 연산 단위 회수
- [x] T010 예약 이름 `_type`, `__actor` 등 → `OG_RESERVED_PROPERTY`
- [ ] T011 이후 생기는 하위 타입·컬럼에 부여 재적용 — 구현, 07 은 부여 **이전**에 있던 하위 타입만 검사
- [x] T012 관계 타입의 `og_edge`, `og_adj` 정책 — 관계 부여가 없는 역할의 INSERT 거부 (10)
- [ ] T012a `og_role_player` 정책 — 구현, 테스트 없음
- [ ] T013 `og_revoke_write(…, ops => NULL)` — 구현, 테스트 없음

## Phase 2 — 스탬프와 감사
- [x] T014 `og_enable_stamping` — `created_by`, `actor`, `source`, `created_at`
- [x] T015 원시 SQL 로 스탬프 위조 불가
- [x] T016 이후 생성되는 하위 타입에 스탬프 상속
- [x] T017 `og_meta(id)`
- [x] T018 감사 `db_user`, `actor`, `source`, `schema_version`; 읽기 역할의 질의도 기록
- [x] T019 쓰기 역할은 `og_audit` 수정·삭제 불가
- [ ] T020 감사 `request_id` — 구현, 값 단언 없음
- [x] T021 `og.reason` 기록 — `og_audit.reason` (07)

## Phase 3 — 행위
- [x] T022 `og_define_action` + `og_grant_action` — 읽기 전용 에이전트가 행위로만 쓴다
- [x] T023 전제 조건 거짓 → `OG_ACTION_REJECTED` (23514)
- [x] T024 파라미터 계약 — 미선언 키, 타입 불일치 → 22023
- [x] T025 실행기 직접 호출 거부, 권한 없는 역할의 실행 거부
- [x] T026 감사 행에 행위 이름
- [x] T027 `og_revoke_action`, `og_drop_action` (10)
- [ ] T027a 행위 재정의 — 구현, 테스트 없음
- [ ] T028 TypeQL 행위 본문에 파라미터 전달 — **미구현**
- [x] T029 본문 오류 시 감사용 행위 이름 복원 — drop guard `ActionGuard` (07)

## Phase 4 — Bolt 컨텍스트
- [x] T030 자동 커밋 `RUN` 의 `tx_metadata.actor/source` → 스탬프와 감사
- [x] T031 명시적 트랜잭션의 `tx_metadata.actor` → 스탬프
- [x] T032 다음 트랜잭션으로 새지 않음
- [x] T033 `tx_metadata` 키 → 설정 매핑 단위 테스트 (`bolt/src/session.rs`, `cargo test`)

## Phase 5 — not yet
- [ ] T034 레지스트리·인접 세그먼트의 읽기 제한 (SEC-08 나머지 절반)
- [ ] T035 릴리스 빌드·비소유자 역할에서의 쓰기 오버헤드 측정, 하네스 포함
- [ ] T036 행위의 버전 관리와 별도 실행 이력
