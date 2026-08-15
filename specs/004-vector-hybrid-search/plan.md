# Implementation Plan: 벡터 및 하이브리드 시맨틱 검색

**Branch**: `004-vector-hybrid-search` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

pgvector를 저장·인덱싱 엔진으로 삼되, **노드와 관계(엣지) 양쪽에 임베딩 슬롯을 부여**하고
그래프 술어가 벡터 탐색 **안으로** 내려가는 하이브리드 검색을 구현한다.

**핵심 설계 결정**

1. **임베딩은 프로퍼티다.** 002의 `vector(N)` 프로퍼티 타입이 001의 타입 테이블에 실제
   `vector` 컬럼을 만든다. 별도 임베딩 저장소가 없다 → 트랜잭션·백업·RLS가 자동 상속된다.
   관계 임베딩이 "1급"인 이유도 이것이다: 엣지 타입 테이블도 똑같이 컬럼을 갖는다.
2. **push-down은 컴파일 타임에 일어난다.** 003이 라벨을 구체 타입 테이블 목록으로 이미
   해소했으므로, 벡터 검색은 `og_data.n_<sub>` 위의 HNSW 인덱스 스캔 + 같은 테이블의 컬럼
   술어가 된다. 사후 필터링이 원천적으로 불가능한 구조다.
3. **선택도 기반 경로 전환**은 PostgreSQL 플래너에 위임한다. 필터가 강하면 인덱스 스캔 →
   비트맵 → 정확 정렬, 약하면 HNSW. 우리가 별도 규칙을 만들지 않는다.

## Technical Context

**Dependencies**: pgvector 0.8+ (HNSW/IVFFlat, `<=>` `<->` `<#>`)

**Testing**: 재현율 측정(정확 탐색 대비), 필터 선택도 스윕, 관계 임베딩 검색

**Performance Goals**: 100만 노드 top-10 p95 50ms, 필터 통과율 0.01%에서 결과 10개 보장

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I | ✅ | pgvector는 표준 확장. 자체 ANN 구현 없음 |
| **V** | ✅ | 노드·관계·경로 모두 임베딩 대상. 계획 통합 push-down |
| VI | ✅ | 벡터 전용 저장 계층 없음 |
| IX | ✅ | 임베딩이 타입 테이블 컬럼 → MVCC 자동 |

## Architecture

**Cypher 표면**

```cypher
-- 노드 시맨틱 검색 (그래프 술어와 한 문장)
MATCH (c:Company)
WHERE c.sector = 'manufacturing'
RETURN c.name, vector.similarity(c.embedding, $q) AS score
ORDER BY score DESC LIMIT 10

-- 관계 시맨틱 검색 — Neo4j가 1급으로 제공하지 않는 부분
MATCH (a)-[r:PARTNERSHIP]->(b)
RETURN r, vector.similarity(r.context_embedding, $q) AS score
ORDER BY score DESC LIMIT 10
```

`vector.similarity(x, y)` → `(1 - (x <=> y))`, `vector.distance` → `<=>`, `vector.l2` → `<->`.
`$q` 파라미터는 003의 타입 힌트 기구가 `::vector` 로 캐스팅한다.

**API**

| 함수 | 역할 |
|------|------|
| `og_add_embedding(graph, type, prop, dims, metric, source_prop)` | 임베딩 슬롯 선언 + HNSW 인덱스 |
| `og_vector_search(graph, type, prop, query, k, filter)` | 노드/관계 공통 top-k |
| `og_similar(id, prop, k)` | 기준 엔티티 유사 검색 |
| `og_stale_embeddings(graph)` | 소스 변경으로 무효화된 임베딩 |
| `og_embedding_stats(graph)` | 개수/차원/stale 비율 |
| `og_hybrid_search(...)` | 벡터 + 그래프 근접 + FTS 결합(RRF/가중합) |

**Staleness**: `property.source_prop` 를 카탈로그에 기록하고, 소스 컬럼 변경 시
`og_data.og_embedding_stale` 에 기록하는 트리거를 타입 테이블에 건다.

## Phasing

| Phase | 내용 |
|-------|------|
| P0 | `vector(N)` 프로퍼티 + HNSW 인덱스 생성, Cypher `vector.*` 함수 |
| P1 | `og_vector_search` / `og_similar` (노드·관계 공통) |
| P2 | staleness 추적, 통계 |
| P3 | 하이브리드 점수(RRF/가중합), FTS 결합 |
| P4 | 경로/서브그래프 임베딩 |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| 재현율 보장을 pgvector `hnsw.ef_search` 튜닝에 의존 | ANN 재현율은 인덱스 파라미터의 함수다. 자체 구현은 원칙 I·V 위반 | 사후 필터링은 스펙이 금지. 정확 탐색 강제는 성능 목표 미달 |
