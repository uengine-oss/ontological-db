# 01. 에이전트 네이티브 인터페이스 — 왜 별도 표면이 필요한가

> **이 문서가 답하는 질문**
> - 사람이 쓰는 DB 인터페이스와 에이전트가 쓰는 인터페이스는 왜 달라야 하는가?
> - spec 008이 정의하는 표면은 무엇으로 구성되고, 그중 무엇이 실제로 동작하는가?
> - 에이전트 루프(스키마 → 작성 → 추정 → 실행 → 교정)는 어떤 함수 호출로 이뤄지는가?
> - 이 표면을 쓸 때 반드시 지켜야 하는 규칙과 하지 말아야 할 것은?

---

## 1. 사실 — 문제 정의

`engine/src/agent/mod.rs` 의 모듈 주석이 전제를 명시한다
([engine/src/agent/mod.rs:1-8](../../engine/src/agent/mod.rs)):

> The entity writing Cypher against this database is increasingly a language
> model, and it fails differently from a human. It invents labels, reverses
> relationship directions and writes accidental cartesian products — confidently.
> So the database owes it three things: an accurate machine-readable schema,
> errors that carry their own correction, and limits that stop a bad query before
> it stops the server.

spec 008 개요도 같은 세 가지를 든다
([specs/008-agent-native-interface/spec.md:13-20](../../specs/008-agent-native-interface/spec.md)):
기계 판독 가능한 스키마 / 교정 가능한 오류 / 답의 출처 / "언제 기준으로 참인가".

LLM의 실패 양식은 사람의 실패 양식과 다르다. 사람은 모르면 문서를 찾지만, LLM은
**확신에 차서 존재하지 않는 레이블을 만들어낸다.** 그래서 "문서를 읽어라"는 해법이
성립하지 않고, 오류 응답 자체가 교정 정보를 실어야 한다.

---

## 2. 사실 — 표면의 구성과 구현 상태

spec 008 plan의 아키텍처 표
([specs/008-agent-native-interface/plan.md:21-31](../../specs/008-agent-native-interface/plan.md))
와 실제 코드를 대조한 결과다.

| 함수 | FR | 구현 위치 | 상태 |
|---|---|---|---|
| `og_schema(graph, token_budget)` | 001-006 | [engine/src/agent/mod.rs:21-113](../../engine/src/agent/mod.rs) | 동작 (통계는 인스턴스 수만) |
| `og_schema_for(graph, question)` | 004 | [engine/src/agent/mod.rs:189-258](../../engine/src/agent/mod.rs) | 동작 (어휘 매칭만) |
| `og_explain_error(graph, query)` | 007-011 | [engine/src/agent/mod.rs:261-292](../../engine/src/agent/mod.rs) | 동작 (분류 로직에 결함 — 03 문서 참조) |
| `og_diagnose_empty(graph, query)` | 010 | [engine/src/agent/mod.rs:339-347](../../engine/src/agent/mod.rs) → [engine/src/cypher/mod.rs:747-803](../../engine/src/cypher/mod.rs) | 동작 (파라미터 미전달 결함) |
| `og_estimate(graph, query)` | 030, 031 | [engine/src/agent/mod.rs:350-397](../../engine/src/agent/mod.rs) | 동작 |
| `og_create_role` / `og_apply_role` | 024-029 | [engine/src/agent/mod.rs:404-441](../../engine/src/agent/mod.rs) | 부분 (`max_rows` 미강제) |
| `og_enable_history` / `og_history` / `og_as_of` | 018-023 | [engine/src/agent/mod.rs:448-526](../../engine/src/agent/mod.rs) | 동작 (valid/transaction time 분리는 미구현) |
| `og_set_source` | 015 | [engine/src/agent/mod.rs:529-545](../../engine/src/agent/mod.rs) | 동작 |
| `og_cypher_provenance` (질의 결과별 기여 노드) | 012-014 | — | **미구현** ([specs/008-agent-native-interface/tasks.md:27-28](../../specs/008-agent-native-interface/tasks.md)) |
| MCP 전용 서버 바이너리 | 032-034 | — | **미구현**. 대신 Neo4j MCP 서버가 Bolt로 붙는다 ([09_mcp_integration.md](09_mcp_integration.md)) |

미구현 항목의 근거는 tasks.md의 미체크 항목이다
([specs/008-agent-native-interface/tasks.md:22-23, 27-28, 34, 37](../../specs/008-agent-native-interface/tasks.md)):
T011(방문 노드 수 상한 주입), T012(반복 실패 질의 속도 제한), T014(결과별 기여 추적),
T015(추론 근거 반환), T019(valid/transaction time 분리), T020(MCP 서버).

---

## 3. 사실 — 루프의 형태

원문 [docs/agents.md:8-22](../../docs/agents.md) 가 그리는 루프를, 실제 함수 호출로 다시 쓰면
다음과 같다. 모든 단계가 **평문 SQL 호출**이라는 점이 이 설계의 핵심이다
(spec 008 FR-034, [specs/008-agent-native-interface/spec.md:281](../../specs/008-agent-native-interface/spec.md)).

```sql
-- 1. 질문에 관련된 스키마만 받는다 (토큰 절약)
SELECT og_schema_for('meeting', '"라일락" 회의실을 어제 예약했던 사람 목록');

-- 2. (에이전트가 Cypher 작성)

-- 3. 구문만 먼저 확인한다 — DB 접근 없음, immutable
SELECT og_cypher_check($$MATCH (m:MeetingRoom) RETURN m.name$$);

-- 4. 실행 전 비용 추정
SELECT og_estimate('meeting', $$MATCH (a:Employee), (b:Employee) RETURN a.name, b.name$$);

-- 5. 실행
SELECT og_cypher('meeting',
  $$MATCH (e:Employee)<-[:RESERVED_BY]-(r:Reservation) RETURN e.name, r.purpose$$,
  '{}'::jsonb);

-- 6a. 오류였다면: 구조화된 교정 정보
SELECT og_explain_error('meeting', $$MATCH (p:Emploee) RETURN p$$);

-- 6b. 0행이었다면: 어느 패턴 단계에서 죽었는가
SELECT og_diagnose_empty('meeting',
  $$MATCH (e:Employee)<-[:RESERVED_BY]-(r:Reservation) RETURN e.name$$);
```

**주의**: 5번과 6번 사이에 트랜잭션 경계가 있다. `og_cypher` 가 실패하면 `error!` 로
트랜잭션이 중단되므로([engine/src/cypher/mod.rs:96-98, 140](../../engine/src/cypher/mod.rs)),
6a/6b는 **새 트랜잭션**에서 호출해야 한다. 같은 트랜잭션에서 이어 호출하면
`current transaction is aborted` 를 받는다.

---

## 4. 결정(Decision) — 왜 이렇게 설계되었는가

### D-1. LLM 호출은 DB 안에 두지 않는다 (예외 1건)

spec 008은 자연어→Cypher 변환을 명시적으로 범위 밖에 둔다
([specs/008-agent-native-interface/spec.md:326-328](../../specs/008-agent-native-interface/spec.md)).
유일한 예외인 `genai.vector.encode` 는 임베딩 호출인데, 이것조차 "Neo4j가 같은 결론에
도달해 같은 이름의 함수를 추가했기 때문"으로 정당화되고
([engine/src/compat/genai.rs:10-11](../../engine/src/compat/genai.rs)), 기본 비활성이다.

### D-2. 오류는 문자열이 아니라 구조체다

`og_explain_error` 는 `{ok, code, message, stage, suggestions}` 를 낸다
([engine/src/agent/mod.rs:264-289](../../engine/src/agent/mod.rs)). `code` 가 안정적이어야
에이전트의 재시도 분기가 성립한다(FR-011).

### D-3. 스키마는 잘라내되, 잘랐다고 말한다

`og_schema` 는 예산 초과 시 `truncated` 객체를 응답에 넣는다
([engine/src/agent/mod.rs:101-111](../../engine/src/agent/mod.rs)). 모듈 주석의 표현:
"an agent given a truncated schema it *knows* is truncated behaves better than one
given a complete schema it cannot afford to read"
([engine/src/agent/mod.rs:17-20](../../engine/src/agent/mod.rs)).

### D-4. 이력이 없으면 현재 값을 반환하지 않고 오류를 낸다

`og_as_of` 는 이력이 없는 엔티티에 대해 명시적으로 `error!` 를 던진다
([engine/src/agent/mod.rs:511-516](../../engine/src/agent/mod.rs)):
"returning the current value instead would be a lie". FR-021의 직접 구현이다.

### D-5. 사용자 값은 SQL 텍스트로 보간하지 않는다

`og_cypher(graph, query, params jsonb)` — 파라미터는 jsonb 하나로 바인딩되고
([engine/src/cypher/mod.rs:84-88](../../engine/src/cypher/mod.rs)), 컴파일된 SQL 안에서
`($1 ->> 'name')` 형태로 참조된다
([engine/src/cypher/compile.rs:1156-1157](../../engine/src/cypher/compile.rs)).
spec 003 FR-026.

---

## 5. 금지(Forbidden) / 필수(Required)

**필수**

- 에이전트에게 Cypher를 생성시키기 전에 `og_schema_for` 또는 `og_schema` 를 먼저 호출할 것.
  스키마 없이 생성한 Cypher는 레이블을 지어낸다.
- 스키마 응답의 `schema_version` 을 캐시 키로 쓸 것
  ([engine/src/agent/mod.rs:24-30, 90](../../engine/src/agent/mod.rs)).
  값이 그대로면 컨텍스트에 유지하고, 바뀌면 다시 받는다.
- 응답에 `truncated` 가 있으면, 그 사실을 프롬프트에 그대로 넘길 것.
- 오류 진단 함수(`og_explain_error`, `og_diagnose_empty`)는 **실패한 트랜잭션 밖에서** 호출할 것.
- 사용자 입력은 반드시 `params` jsonb로 전달할 것. 질의 문자열에 보간 금지.

**금지**

- `og_explain_error` 의 `{"ok": true}` 를 "이 질의는 결과가 있다"로 해석하지 말 것.
  존재하지 않는 레이블은 컴파일에 성공하고 0행을 반환한다
  ([engine/src/catalog/types.rs:160-175](../../engine/src/catalog/types.rs),
  [engine/src/cypher/compile.rs:709-715](../../engine/src/cypher/compile.rs)).
  상세는 [03_correctable_errors.md](03_correctable_errors.md) 4절.
- `og_estimate` 의 `would_run: true` 를 안전 보증으로 쓰지 말 것. `advice` 배열이
  비었다는 뜻일 뿐이고, 판정 기준은 세 개의 하드코딩된 임계값이다
  ([engine/src/agent/mod.rs:374-388](../../engine/src/agent/mod.rs)).
- `og_apply_role` 이 적용한 `og.max_rows` 가 행 수를 제한한다고 가정하지 말 것.
  저장소 어디에서도 이 GUC를 읽지 않는다 (근거: [08_guardrails_and_roles.md](08_guardrails_and_roles.md) 3절).
- 에이전트에게 `og_set_setting` 실행 권한을 주지 말 것. 임베딩 엔드포인트 URL을
  바꿀 수 있게 되고, 이는 SSRF 경로다
  ([engine/src/compat/genai.rs:55-63](../../engine/src/compat/genai.rs), 상세는
  [05_embedding_pipeline.md](05_embedding_pipeline.md) 6절).

---

## 6. 참고

- 원문: [docs/agents.md](../../docs/agents.md)
- 함수 계약: [docs/api.md](../../docs/api.md) "Agents — spec 008" 절 (169-185행)
- 스펙: [specs/008-agent-native-interface/spec.md](../../specs/008-agent-native-interface/spec.md),
  [specs/008-agent-native-interface/plan.md](../../specs/008-agent-native-interface/plan.md),
  [specs/008-agent-native-interface/tasks.md](../../specs/008-agent-native-interface/tasks.md)

<!-- affects: llm, api, backend -->
<!-- requires-update: 02_api/00_index.md, 05_llm/03_correctable_errors.md -->
