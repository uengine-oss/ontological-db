# Implementation Plan: 에이전트 네이티브 인터페이스

**Branch**: `008-agent-native-interface` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

에이전트가 정확한 Cypher를 만들도록 **정보를 주고**, 틀렸을 때 **고칠 수 있게 하고**,
폭주하면 **막는다**. LLM 호출은 하지 않는다.

**핵심 설계 결정**

1. **스키마 요약은 토큰 예산을 받는다.** 타입 수가 많으면 인스턴스 수 기준으로 중요도를
   정렬해 잘라내고, 잘랐다는 사실을 응답에 표시한다.
2. **오류는 구조체다.** 안정적 코드 + 후보 제안 + 사람용 메시지. 이미 002/003의 오류
   경로가 편집거리 후보를 붙이고 있다(`nearest_type_names`).
3. **출처는 선택적이다.** 켜지 않으면 오버헤드 0 — 컴파일러가 아예 다른 SQL을 낸다.
4. **가드레일은 역할 단위.** `og_catalog.agent_role` 에 상한을 두고 세션 GUC로 강제한다.

## Architecture

| 함수 | 스펙 |
|------|------|
| `og_schema(graph, token_budget)` | FR-001..006 — 기계 판독 스키마 |
| `og_schema_for(graph, question)` | FR-004 — 질문 관련 부분집합 |
| `og_explain_error(graph, query)` | FR-007..011 — 구조화 오류 + 후보 |
| `og_diagnose_empty(graph, query)` | FR-010 — 어느 단계에서 0이 되었는가 |
| `og_cypher_provenance(graph, query, params)` | FR-012..017 — 기여 노드/엣지/경로 |
| `og_history(id)` / `og_as_of(graph, query, ts)` | FR-018..023 — 시점 질의 |
| `og_create_role(name, limits jsonb)` | FR-024..029 — 리소스 상한 |
| `og_estimate(graph, query)` | FR-030,031 — dry-run 비용 |
| MCP 서버 (`portal/mcp`) | FR-032..034 |

**시점 질의**: `og_data.og_history` 에 valid_from/valid_to/txid를 기록하는 트리거를
타입 테이블에 선택적으로 건다(기본 off — 비용 때문). `og_as_of` 는 현재 그래프에
히스토리 델타를 역적용하지 않고, **히스토리에서 해당 시점 상태를 재구성한 임시 뷰**로
질의를 실행한다.

**리소스 상한**: `statement_timeout`, `work_mem`, `og.max_visited` 를 역할별로 설정.
`og.max_visited` 는 컴파일된 SQL에 `LIMIT` 가드를 삽입해 강제한다.

## Constitution Check

| 원칙 | 상태 |
|------|------|
| **VIII** | ✅ 본 스펙이 원칙 VIII의 구현체 |
| I | ✅ 전부 SQL 함수 + 표준 GUC |
| IX | ✅ 히스토리도 일반 테이블 → 트랜잭션 참여 |

## Phasing

| Phase | 내용 |
|-------|------|
| P0 | `og_schema`, 구조화 오류, 감사 로그 |
| P1 | dry-run 비용 추정, 리소스 상한 |
| P2 | 출처 추적 |
| P3 | 히스토리 / 시점 질의 |
| P4 | MCP 서버 |
