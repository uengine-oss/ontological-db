# Ontological — 스펙 개요

PostgreSQL 확장으로 구현하는, AI 에이전트 시대를 위한 Cypher 중심 온톨로지 그래프 데이터베이스.

거버넌스는 [.specify/memory/constitution.md](../.specify/memory/constitution.md) 를 따른다.
모든 `plan.md` 는 Constitution Check 섹션을 포함해야 한다.

## 스펙 목록

| # | 스펙 | 한 줄 요약 | 벤치마킹 대상 |
|---|------|-----------|--------------|
| 001 | [네이티브 그래프 저장 엔진](001-graph-storage-engine/spec.md) | 인접 세그먼트 + 타입 기반 슬롯 저장. AGE의 힙테이블+agtype 구조를 대체 | Neo4j 저장 계층 |
| 002 | [온톨로지 타입 시스템](002-ontology-type-system/spec.md) | entity/relation/attribute 타입, role, 다중 상속, **구간 라벨링 상속 인덱스** | TypeDB |
| 003 | [네이티브 Cypher 질의 엔진](003-cypher-query-engine/spec.md) | 자체 파서 → 논리계획 → Custom Scan. 문자열 함수 래퍼 거부 | openCypher / Neo4j |
| 004 | [벡터·하이브리드 검색](004-vector-hybrid-search/spec.md) | pgvector 위에서 **노드·관계·경로** 임베딩. 계획 통합 하이브리드 검색 | Neo4j Vector Index |
| 005 | [PostgreSQL/Supabase 상호운용](005-postgres-supabase-interop/spec.md) | SQL↔Cypher 조인, 기존 테이블 매핑, RLS, PostgREST, Realtime | Supabase 스택 |
| 006 | [시맨틱 웹 어댑터](006-semantic-web-adapters/spec.md) | RDF/OWL/SPARQL/SHACL/GraphQL — 어댑터만, 코어는 하나 | Jena, GraphDB |
| 007 | [분산 클러스터](007-distributed-cluster/spec.md) | 그래프 인지 샤딩 + 연산 push-down. 단일 노드 성능 불변 | Neo4j Fabric, Citus |
| 008 | [에이전트 네이티브 인터페이스](008-agent-native-interface/spec.md) | 스키마 introspection, 교정 가능한 오류, provenance, 시점 질의, MCP | — (신규 영역) |
| 009 | [벤치마크·적합성 하네스](009-benchmark-conformance/spec.md) | LDBC SNB, openCypher TCK, AGE/Neo4j 자동 비교, CI 회귀 게이트 | LDBC |
| 010 | [TypeQL 질의 표면](010-typeql-query-surface/spec.md) | TypeDB 3.x TypeQL 을 두 번째 1급 질의 언어로. 같은 카탈로그·저장·트랜잭션 | TypeDB / TypeQL |
| 011 | [Bolt 프로토콜 게이트웨이](011-bolt-protocol-gateway/spec.md) | Neo4j 드라이버가 URI만 바꿔 접속. 코어 밖 게이트웨이, 질의 경로는 여전히 하나 | Neo4j Bolt 4.4 |

## 의존 관계

```
001 저장 엔진 ──┬──> 003 질의 엔진 ──┬──> 004 벡터 검색
                │                     ├──> 005 상호운용
002 타입 시스템 ┘                     ├──> 006 어댑터
                                      ├──> 007 분산
                                      ├──> 010 TypeQL 표면
                                      └──> 011 Bolt 게이트웨이

002 + 003 + 004 + 005 ──> 008 에이전트 인터페이스

009 벤치마크 ──> 전 스펙의 Success Criteria 를 검증
```

- **001 + 002** 가 기반. 둘 다 없으면 003이 성립하지 않는다.
- **003** 이 나머지 전부의 실행 계층이다. 006의 SPARQL, 004의 벡터 연산자, 007의 분산 계획이
  모두 003의 논리 계획 위에 올라간다. 따라서 003의 계획 표현은 **언어 중립적·분산 확장 가능**
  해야 한다.
- **009** 는 다른 스펙의 성공 기준을 측정하는 인프라이므로 001~003과 병행 착수해야 한다.
- **010 · 011** 은 003 위의 표면이다. 010은 언어를, 011은 전송을 추가하며, 어느 쪽도
  코어 의미론을 갖지 않는다(헌법 원칙 VI). 011은 003의 FR-024a를 대체한다.

## 권장 진행 순서

1. **009** 하네스 골격 (측정 없이 개발하면 001의 설계 의도를 잃는다)
2. **001 + 002** 병행 (002의 슬롯 레이아웃이 001의 물리 구조를 결정)
3. **003** 질의 엔진
4. **005** 상호운용 (여기까지가 최소 사용 가능 제품)
5. **004** 벡터 → **008** 에이전트 인터페이스 (AI 차별점)
6. **006** 어댑터, **007** 분산 (확장 단계)

## 각 스펙의 다음 단계

```bash
export SPECIFY_FEATURE=001-graph-storage-engine
# /speckit-clarify  → 모호한 지점 확정
# /speckit-plan     → plan.md (Constitution Check 필수)
# /speckit-tasks    → tasks.md
```
