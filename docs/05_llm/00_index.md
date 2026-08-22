# 05_llm — LLM·에이전트가 소비하는 데이터베이스

> **이 문서가 답하는 질문**
> - 이 카테고리는 "LLM 앱 문서"인가, 아니면 다른 것인가?
> - LLM 에이전트가 이 데이터베이스를 쓸 때 실제로 호출하는 함수는 무엇인가?
> - RAG 파이프라인에서 이 DB가 담당하는 계층은 어디까지이고, 어디부터는 아닌가?
> - 각 세부 문서는 무엇을 다루며, 어떤 순서로 읽어야 하는가?

---

## 1. 사실 — 이 카테고리의 범위

Ontological은 **LLM을 호출하는 애플리케이션이 아니다.** LLM/에이전트가 *소비하는*
데이터베이스다. 이 구분은 스펙에 명시되어 있다.

- spec 008: "LLM 호출(자연어 → Cypher 변환 자체)은 본 시스템의 범위가 아니다.
  시스템은 에이전트가 정확한 질의를 만들 수 있도록 **정보와 안전장치를 제공**한다"
  ([specs/008-agent-native-interface/spec.md:326-328](../../specs/008-agent-native-interface/spec.md))
- spec 004: "임베딩 **생성**(모델 호출)은 본 시스템의 범위가 아니다"
  ([specs/004-vector-hybrid-search/spec.md:269-270](../../specs/004-vector-hybrid-search/spec.md))

이 원칙에는 **정확히 하나의 예외**가 있다. `genai.vector.encode` 는 데이터베이스
백엔드에서 외부 임베딩 엔드포인트로 HTTP 요청을 보낸다
([engine/src/compat/genai.rs:139-149](../../engine/src/compat/genai.rs)). 확장 전체에서
유일한 아웃바운드 네트워크 호출이고, 기본값이 꺼져 있다
([engine/src/compat/genai.rs:101-107](../../engine/src/compat/genai.rs)).

따라서 이 카테고리가 다루는 것은 네 덩어리다.

| 덩어리 | 스펙 | 소스 |
|---|---|---|
| 에이전트 네이티브 인터페이스 (스키마 인트로스펙션, 교정 가능한 오류, dry-run, 히스토리, 감사, 역할) | 008 | [engine/src/agent/mod.rs](../../engine/src/agent/mod.rs) (545줄) |
| 벡터/하이브리드 시맨틱 검색 = RAG 검색 계층 | 004 | [engine/src/vector/mod.rs](../../engine/src/vector/mod.rs) (442줄) |
| 외부 임베딩 호출 `genai.vector.encode` | 004/008 접점 | [engine/src/compat/genai.rs](../../engine/src/compat/genai.rs) (177줄) |
| Neo4j MCP 서버가 Bolt 게이트웨이를 통해 그대로 동작 | 008 FR-032, 011 | [examples/meeting-rooms/](../../examples/meeting-rooms/) |

---

## 2. 사실 — RAG 파이프라인에서 이 DB의 위치

전형적인 Agentic RAG 파이프라인의 노드를 기준으로, 이 데이터베이스가 **제공하는 것**과
**제공하지 않는 것**을 분리하면 다음과 같다.

| RAG 노드 | 이 DB가 제공하는가 | 근거 |
|---|---|---|
| Query Analyze / Intent | 부분 — `og_schema_for(graph, question)` 가 질문의 어휘로 관련 타입만 선별 | [engine/src/agent/mod.rs:189-258](../../engine/src/agent/mod.rs) |
| Retrieve (Vector) | ✅ `og_vector_search`, `og_similar` | [engine/src/vector/mod.rs:94-202](../../engine/src/vector/mod.rs) |
| Retrieve (Keyword/FTS) | ✅ 단, Cypher 표면 `db.index.fulltext.queryNodes` 로만 | [engine/src/compat/procs.rs:203-240](../../engine/src/compat/procs.rs) |
| Rank Fusion (RRF) | ✅ 단, **벡터 순위 + 그래프 근접**의 융합이며 FTS와의 융합은 미구현 | [engine/src/vector/mod.rs:263-278](../../engine/src/vector/mod.rs), [specs/004-vector-hybrid-search/tasks.md:29](../../specs/004-vector-hybrid-search/tasks.md) |
| Reranking (cross-encoder 등) | ❌ 없음 | 코드에 리랭킹 단계 부재 |
| Groundedness / Sufficiency Eval | ❌ 없음 (질의 전 검증은 `og_estimate`, 답변 근거성 검증 훅은 없음) | [engine/src/agent/mod.rs:350-397](../../engine/src/agent/mod.rs) |
| Query Rewrite | ❌ 없음 — 재작성은 에이전트의 몫. DB는 재작성에 **필요한 정보**만 준다 | `og_explain_error`, `og_diagnose_empty` |
| Generate | ❌ 범위 밖 | spec 008 Assumptions |
| Hallucination Check | ❌ 없음. 대신 **근거 추적** 재료를 제공 | `og_set_source`, `og_history`, `og_as_of` |
| Guardrails | 부분 — `og_create_role` / `og_apply_role` | [engine/src/agent/mod.rs:404-441](../../engine/src/agent/mod.rs) |
| Audit | 부분 — `og_data.og_audit`, 단 Cypher/TypeQL 경로만 | [engine/src/cypher/mod.rs:122-135](../../engine/src/cypher/mod.rs) |

**결정(Decision)**: 이 DB는 "검색 + 검증 재료" 계층이다. 자기교정 루프(self-correction
loop)의 **판정 노드는 애플리케이션이 구현**한다. DB는 그 판정에 필요한 신호
(오류 코드, 후보 목록, 단계별 행 수, 비용 추정치, 유사도 점수, 출처, 시점)를 낸다.

---

## 3. 문서 지도

| # | 문서 | 무엇을 답하는가 |
|---|---|---|
| 01 | [01_agent_native_interface.md](01_agent_native_interface.md) | 왜 에이전트용 인터페이스가 별도로 필요한가, 전체 루프의 형태 |
| 02 | [02_schema_introspection.md](02_schema_introspection.md) | `og_schema` / `og_schema_for` — 토큰 예산 로직의 실제 동작과 한계 |
| 03 | [03_correctable_errors.md](03_correctable_errors.md) | `og_explain_error` / `og_diagnose_empty` — 실제 오류 코드 목록과 재시도 루프 설계 |
| 04 | [04_dry_run_and_estimate.md](04_dry_run_and_estimate.md) | `og_estimate` / `og_cypher_check` — 실행 전 비용 추정과 검증 |
| 05 | [05_embedding_pipeline.md](05_embedding_pipeline.md) | 임베딩 생성→저장→갱신, `genai.vector.encode` 계약, stale 추적, 실패 모드 |
| 06 | [06_retrieval_and_rrf.md](06_retrieval_and_rrf.md) | 벡터 검색과 하이브리드 RRF의 실제 공식, 필터 푸시다운, 관계 임베딩 |
| 07 | [07_grounding_and_provenance.md](07_grounding_and_provenance.md) | `og_set_source` / `og_history` / `og_as_of` — 답변 근거 추적 |
| 08 | [08_guardrails_and_roles.md](08_guardrails_and_roles.md) | `og_create_role` / `og_apply_role` / `og_add_rule` — 허용/금지 경계 |
| 09 | [09_mcp_integration.md](09_mcp_integration.md) | Neo4j MCP 서버 연결 절차 (복사-붙여넣기 가능) |
| 10 | [10_improvements_llm.md](10_improvements_llm.md) | ★ LLM/RAG 계층 개선 포인트 (`LLM-nn`) |

원문 영문 문서: [docs/agents.md](../../docs/agents.md), [docs/api.md](../../docs/api.md).
코드와 불일치하는 부분은 이 카테고리 문서가 코드 쪽을 정답으로 삼아 기록한다
(구체적 불일치는 [03_correctable_errors.md](03_correctable_errors.md) 4절 참조).

---

## 4. 이 카테고리 전체에 적용되는 규칙

**필수(Required)**

- 이 문서군에서 함수 동작을 인용할 때는 반드시 `파일:라인` 근거를 붙인다.
- 스펙 요구사항(FR/SC)과 **실제 구현**을 구분해 쓴다. 스펙에 있으나 미구현인 항목은
  "미구현"으로 명시한다.
- 수치(토큰 예산 환산, RRF 상수, 타임아웃)는 코드에서 읽은 값만 쓴다.

**금지(Forbidden)**

- "이 DB가 환각을 막아준다"는 식의 서술 금지. 이 DB는 환각 **검출에 필요한 재료**를
  낼 뿐이고, 판정 노드는 없다.
- spec 008/004의 Success Criteria(SC-001, SC-003 등)를 달성된 사실처럼 쓰지 말 것.
  저장소에 그 측정을 수행하는 하네스는 없다 — `bench/harness.py` 에는 recall 측정
  코드가 없고([bench/harness.py](../../bench/harness.py) 내 "recall" 문자열 부재),
  ANN vs exact 비교는 3행짜리 회귀 테스트 한 건이 전부다
  ([engine/tests/sql/03_vector_agent_rdf.sql:34-36](../../engine/tests/sql/03_vector_agent_rdf.sql)).
- `docs/README.md` 및 기존 영문 `docs/*.md` 수정 금지.

<!-- affects: llm, api, backend -->
<!-- requires-update: 02_api/00_index.md -->
