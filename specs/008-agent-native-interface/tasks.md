# Tasks: 에이전트 네이티브 인터페이스

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — schema & errors
- [x] T001 `og_schema` — 기계 판독 스키마 (타입 계층, role, 프로퍼티, 통계)
- [x] T002 토큰 예산 기반 축약 + 축약 사실 명시
- [x] T003 `og_schema_for` — 질문 관련 부분집합
- [x] T004 `schema_version`을 캐시 무효화 키로 노출
- [x] T005 편집거리 기반 후보 제안 (Rust 구현, 외부 확장 의존 없음)
- [x] T006 `og_explain_error` — 안정적 오류 코드 + 구조화 상세
- [x] T007 `og_diagnose_empty` — 패턴 단계별 행 수 추적
- [x] T008 감사 로그 (`og_audit`) — 모든 질의 기록

## Phase 1 — guardrails
- [x] T009 `og_create_role` / `og_apply_role` — 역할별 리소스 상한
- [x] T010 `og_estimate` — dry-run 비용 + 구체적 개선 제안
- [ ] T011 방문 노드 수 상한을 컴파일된 SQL에 주입
- [ ] T012 반복 실패 질의 속도 제한

## Phase 2 — provenance
- [x] T013 `og_set_source` — 출처 메타데이터 부착
- [ ] T014 질의 결과별 기여 노드/엣지/경로 추적
- [ ] T015 추론 근거 반환

## Phase 3 — temporal
- [x] T016 `og_enable_history` — 타입 단위 opt-in 트리거
- [x] T017 `og_history` — 엔티티 변경 이력
- [x] T018 `og_as_of` — 이력 없으면 현재 값 대신 **오류**
- [ ] T019 유효 시각 / 기록 시각 분리 질의

## Phase 4 — MCP
- [ ] T020 MCP 서버 (포탈 HTTP API가 동일 기능을 이미 노출)
