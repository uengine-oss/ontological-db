# Tasks: 벤치마크 및 적합성 하네스

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — comparison harness
- [x] T001 결정적 그래프 생성기
- [x] T002 시스템 어댑터: ontological / ontological_raw / age / cte
- [x] T003 **정확성 게이트** — 답이 다르면 타이밍 무효 처리
- [x] T004 단일 세션 내 측정 (프로세스 시작 오버헤드 제거)
- [x] T005 논리 페이지 접근 수 측정 (`EXPLAIN BUFFERS`)
- [x] T006 JSON 결과 + 사람이 읽는 요약
- [x] T007 `og_check_integrity`를 결과에 포함
- [x] T008 `--compare-baseline` 회귀 게이트 (20% 임계)
- [x] T009 Apache AGE 1.5.0 실측 비교 완료 (README/bench 결과)

## Phase 1 — conformance (not started)
- [ ] T010 openCypher TCK 러너
- [ ] T011 통과율 기준선 + 회귀 차단
- [ ] T012 지원 기능 매트릭스 자동 생성

## Phase 2 — LDBC (not started)
- [ ] T013 LDBC SNB 생성기 연동
- [ ] T014 Interactive / BI 질의 집합
- [ ] T015 스케일 팩터별 성능 곡선

## Phase 3 — CI (not started)
- [ ] T016 PR 축약 벤치마크
- [ ] T017 통계적 회귀 판정 (오탐 억제)

## Phase 4 — durability (not started)
- [ ] T018 장애 주입 (강제 종료, 디스크 가득참)
- [ ] T019 장시간 스트레스 + 누수 탐지
- [ ] T020 에이전트 정확도 평가 세트
