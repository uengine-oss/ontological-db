# Tasks: PostgreSQL/Supabase 상호운용

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — SQL bridge
- [x] T001 관계형 뷰 (`og_node_view`, `og_edge_view`, 카탈로그 뷰)
- [x] T002 `og_cypher_sql` 결과를 SQL에 임베딩 가능
- [x] T003 결과가 표준 jsonb — 전용 클라이언트 불필요
- [x] T004 `og_cypher_json` — PostgREST RPC 진입점

## Phase 1 — security
- [x] T005 `og_enable_rls` — 타입 계층 전체에 정책 적용
- [x] T006 트래버설 중간 노드에도 RLS 적용 (조인이 강제)
- [x] T007 `og_interop_report` — 보안·매핑 현황
- [ ] T008 멀티테넌트 격리 자동 테스트 100종

## Phase 2 — mapping
- [x] T009 `og_map_table` — 기존 테이블을 노드 타입으로 (복제 0)
- [x] T010 `og_materialize_mapping` — 네이티브 저장으로 물리화
- [ ] T011 외래키 → 관계 자동 매핑
- [ ] T012 쓰기 가능 매핑

## Phase 3 — operations
- [x] T013 pg_dump/restore 정합 (일반 릴레이션)
- [ ] T014 Supabase 로컬 스택 종단 테스트
- [ ] T015 변경 이벤트 스트림 / Realtime 연동
