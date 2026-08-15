# Implementation Plan: PostgreSQL/Supabase 상호운용

**Branch**: `005-postgres-supabase-interop` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

그래프가 사일로가 되지 않게 하는 계층. 세 갈래로 구현한다.

1. **그래프 → SQL**: 컴파일된 Cypher가 애초에 SQL이므로 `og_cypher_sql()` 결과를 그대로
   서브쿼리·뷰·CTE에 넣을 수 있다. 여기에 `og_node_view`/`og_edge_view` 관계형 뷰를 더한다.
2. **SQL → 그래프**: 기존 테이블을 노드/엣지로 해석하는 매핑 선언. 데이터 복제 없이
   뷰로 노출하고, 필요 시 네이티브 저장으로 물리화한다.
3. **Supabase**: RLS 정책이 트래버설 도중에도 적용되도록 하고, PostgREST RPC로 노출한다.

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I | ✅ | 본 스펙이 원칙 I의 검증 지점. superuser 기능 0 |
| II | ✅ | Cypher 결과가 표준 jsonb/컴포지트 → SQL 조인 가능 |
| VIII | ✅ | 감사 로그(`og_audit`)가 여기서 강제됨 |
| IX | ✅ | pg_dump/복제 정합성 테스트가 품질 게이트 |

## Architecture

**RLS 적용 지점**

핵심 통찰: 컴파일된 Cypher가 참조하는 것은 `og_data.n_<tid>` 등 **일반 테이블**이다.
따라서 그 테이블에 RLS 정책을 걸면 **트래버설 중간 노드에도 자동으로 적용된다** — 별도
강제 코드가 필요 없다. 이것이 확장이 아닌 포크였다면 직접 구현해야 했을 부분이다.

단 하나의 구멍: `og_data.og_adj` 는 이웃 id만 담으므로 정책이 걸리지 않는다. 그래서
컴파일러는 항상 대상 노드 뷰와 조인하며(`n2.id = u.nbr`), 그 조인이 RLS를 통과시킨다.
즉 **접근 불가 노드를 경유하는 경로는 결과에서 사라진다**(spec FR-013).

| 함수 | 역할 |
|------|------|
| `og_enable_rls(graph, type, policy_expr)` | 타입 테이블 + 하위 타입 전체에 정책 적용 |
| `og_map_table(graph, table, type, id_col, prop_map)` | 기존 테이블 → 노드 타입 매핑 |
| `og_map_fk(graph, table, rel_type, src_col, dst_col)` | 외래키 → 관계 매핑 |
| `og_materialize_mapping(graph, type)` | 매핑을 네이티브 저장으로 물리화 |
| `og_cypher_json(graph, query, params)` | PostgREST RPC 진입점 (jsonb 배열 반환) |

## Phasing

| Phase | 내용 |
|-------|------|
| P0 | 관계형 뷰, `og_cypher_sql` SQL 임베딩, PostgREST RPC |
| P1 | RLS 헬퍼 + 멀티테넌트 격리 테스트 |
| P2 | 기존 테이블 매핑(읽기 전용) |
| P3 | 매핑 물리화, 쓰기 가능 매핑 |
| P4 | 변경 이벤트 스트림 |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| 논리 복제에서 그래프 구조 표현 미완 | 인접 세그먼트는 배열이라 논리 디코딩 표현이 자명하지 않다. v1은 **물리 복제 완전 지원 + 변경 이벤트 스트림**으로 대체 | 스펙 Assumptions가 이 대체를 명시적으로 허용 |
