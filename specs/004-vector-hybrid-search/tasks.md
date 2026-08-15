# Tasks: 벡터 및 하이브리드 검색

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — embeddings as properties
- [x] T001 `vector(N)` 프로퍼티 타입 — `catalog/types.rs::map_data_type`
- [x] T002 `og_add_embedding` — 노드/관계 공통, HNSW 인덱스 생성
- [x] T003 차원 불일치 시 기대 차원을 명시한 오류
- [x] T004 Cypher `vector.similarity` / `.distance` / `.l2` → pgvector 연산자

## Phase 1 — search
- [x] T005 `og_vector_search` — 노드/관계 공통 top-k
- [x] T006 `og_similar` — 기준 엔티티 유사 검색 (자기 자신 제외)
- [x] T007 그래프 술어 push-down (같은 릴레이션 위 인덱스 + 컬럼 술어)
- [x] T008 `og_vector_search_exact` — 재현율 측정용 정확 탐색

## Phase 2 — lifecycle
- [x] T009 `source_prop` 기록 + `og_stale_embeddings`
- [x] T010 `og_mark_embedded` — 소스 해시 기록
- [x] T011 `og_embedding_stats`

## Phase 3 — hybrid
- [x] T012 `og_hybrid_search` — RRF로 벡터 순위 + 그래프 근접 결합
- [x] T013 구성 점수 개별 반환 (`vector_score`, `graph_score`)
- [ ] T014 PostgreSQL 전문검색(FTS) 결합
- [ ] T015 경로/서브그래프 임베딩
