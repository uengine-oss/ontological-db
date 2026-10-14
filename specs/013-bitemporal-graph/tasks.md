# Tasks: 이중 시간 그래프

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-09-30

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다
(`engine/tests/sql/09_bitemporal.sql`, `engine/tests/sql/10_evolution_temporal_coverage.sql`,
`tests/bolt-context/run.py`).
체크되지 않은 항목은 착수하지 않았거나, 구현되었지만 테스트가 없는 것이다. 이 스펙은 [008](../008-agent-native-interface/tasks.md) T019 를 대체한다.

## Phase 0 — 캡처
- [x] T001 `og_enable_temporal` — `__valid`, `__sys_from`, `h_<tid>`, 트리거 `og_b_temporal`
- [x] T002 생성 시 `og.valid_from` 으로 유효 구간 시작
- [x] T003 생성 시 `og.valid_to` (10)
- [x] T004 갱신: 옛 믿음 닫기 + 과거 유효 구간 재기록 (`u`, `v`)
- [x] T005 정정: `og.valid_from = '-infinity'`
- [x] T006 `OG_TEMPORAL_RANGE` — 유효 구간 끝 이후 시각의 갱신 거부 (10)
- [x] T007 원시 SQL `UPDATE` 도 버전 기록
- [x] T008 쓰기 역할은 이력 테이블 DML 불가, 그래도 쓰기는 캡처됨
- [x] T009 활성화가 이후 생성되는 하위 타입에 상속
- [x] T010 저장 테이블의 컬럼 추가가 이력 테이블에 반영 (`sync_history`)
- [ ] T011 보이는 변화 없는 갱신은 이력 없음 (`og__visible`) — 구현, 전용 단언 없음

## Phase 1 — 시점 질의
- [x] T012 `og.valid_at` — 유효 시각 질의 (2019/2021/2023/2024/2025)
- [x] T013 `og.system_at` — 기록 시각 질의 (정정 전, 인상 기록 전, 퇴사 기록 전)
- [x] T014 삭제된 노드: 현재 없음, 이전 기록 시각에 있음
- [x] T015 시점이 달라도 컴파일된 SQL 동일
- [x] T016 temporal 관계 타입의 단일 홉이 간선 테이블(현재 + 이력)을 시점으로 읽음
- [x] T017 미래 유효 시각의 변경이 그때까지의 값을 숨기지 않음 — 열린 이력 행(`__sys_to = ∞`)이
      현재의 일부 (09: 30 일 뒤 인상 → 지금은 옛 급여 1행, 60 일 뒤 시점은 새 급여)

## Phase 2 — 거부 가드
- [x] T018 레이블 없는 노드 패턴 → `OG_TEMPORAL_UNSUPPORTED`
- [x] T019 temporal 관계 위 가변 길이 경로 → 같은 오류
- [x] T020 TypeQL → 같은 오류
- [x] T021 단일 타입이 아닌 관계 변수 → 같은 오류 (10)

## Phase 3 — Bolt
- [x] T022 `tx_metadata.validFrom` → `og.valid_from` (자동 커밋 `RUN`)
- [x] T023 `tx_metadata.validAt` / `valid_at` → 시점 읽기 (자동 커밋, 명시적 트랜잭션)
- [x] T024 설정은 트랜잭션 로컬 — 다음 트랜잭션에 새지 않음
- [ ] T025 `systemAt` / `validTo` 키 — 매핑은 구현(bolt 단위 테스트는 키 변환만 검사), 종단 테스트 없음

## Phase 4 — 008 레거시 경로
- [x] T026 `og_as_of` 삭제 이후 NULL, 같은 트랜잭션 안의 동시각 변경은 `hist_id` 순서로 (10)
- [ ] T026a 하위 타입 캡처, `og_capture_history` SECURITY DEFINER — 구현, 회귀 테스트 없음

## Phase 5 — not yet
- [ ] T027 가변 길이 경로의 시점 질의
- [ ] T028 레이블 없는 노드 스캔의 시점 질의
- [ ] T029 TypeQL 시점 질의
- [ ] T030 보존 정책 (008 FR-023)
- [ ] T031 과거 유효 구간의 소급 편집
- [ ] T032 쓰기 오버헤드 측정 (008 SC-011)
