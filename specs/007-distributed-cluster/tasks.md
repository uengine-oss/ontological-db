# Tasks: 분산 클러스터

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — read replicas (구현됨)
- [x] T001 모든 그래프 구조가 일반 힙 릴레이션 → 스트리밍 복제 자동 동작
- [x] T002 `og_check_integrity`로 대기 서버 정합성 검증 가능
- [x] T003 식별자에 shard 비트 예약 (재분배 후에도 local_id 불변)

## Phase 1 — sharding (설계 확정, 미구현)
- [ ] T004 `og_catalog.placement` — 배치 정책
- [ ] T005 postgres_fdw 기반 원격 실행
- [ ] T006 컴파일된 SQL의 원격 push-down + 코디네이터 병합
- [ ] T007 분산 트랜잭션 2PC
- [ ] T008 노드 장애 시 조용한 부분 결과 금지 (명시적 오류)
- [ ] T009 온라인 재분배

## Phase 2 — graph-aware partitioning (미착수)
- [ ] T010 엣지 컷 최소화 배치
- [ ] T011 슈퍼노드 인접 분할
- [ ] T012 분산 벡터 top-k 병합

> **미구현을 문서화하는 것이 부분 구현보다 정직하다.** 2PC 없는 분산 쓰기는
> 헌법 원칙 IX(ACID 비협상)를 조용히 위반한다 — plan.md Complexity Tracking 참조.
