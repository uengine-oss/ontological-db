# 에이전트 · 인트로스펙션 API

> **이 문서가 답하는 질문**
> - LLM이 이 데이터베이스에 대해 물어볼 수 있는 것은 정확히 무엇인가?
> - `og_schema`의 `token_budget`은 어떻게 계산되는가?
> - `og_explain_error`가 실제로 감지할 수 있는 오류는 무엇인가?
> - `og_apply_role`의 제한 중 실제로 작동하는 것은?
> - 히스토리와 프로버넌스는 어떻게 켜고 읽는가?

---

## 1. 결정(Decision) — 왜 별도 표면이 있는가

전제: 이 데이터베이스에 Cypher를 쓰는 주체가 점점 언어 모델이고, 그것은 사람과
**다르게 실패한다**. 라벨을 지어내고, 관계 방향을 뒤집고, 우발적 카테시안 곱을
쓴다 — 자신 있게. 그래서 데이터베이스가 세 가지를 빚진다
([engine/src/agent/mod.rs:3](../../engine/src/agent/mod.rs#L3)):

1. 정확한 **기계 판독 스키마**
2. **교정 방법을 스스로 담은 오류**
3. 나쁜 질의가 서버를 멈추기 전에 멈추는 **한계**

---

## 2. 스키마 인트로스펙션

### `og_schema(graph text, token_budget int4 DEFAULT NULL) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:21](../../engine/src/agent/mod.rs#L21) · 휘발성: `STABLE` · 병렬: 기본값(`PARALLEL UNSAFE`)

**무엇을 하는가**: 그래프 전체 스키마를 기계 판독 JSON으로 준다(spec 008 FR-001..FR-006).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `token_budget` | `int4` | 선택 | `NULL` | 대략의 토큰 예산. `NULL`이면 전체 반환 |

**`token_budget` 계산 (코드 그대로, [agent/mod.rs:62](../../engine/src/agent/mod.rs#L62))**

```
cap = max(token_budget / 30, 8)
```

타입 설명 하나를 **30토큰**으로 잡는 의도적 과소 추정이다 — 예산을 넘기느니
조금 적게 주는 편이 낫다. 타입은 **인스턴스 수 내림차순**으로 정렬되어 있고
([agent/mod.rs:42](../../engine/src/agent/mod.rs#L42)) 앞에서 `cap`개만 취한다.

**반환 구조**

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `schema_version` | int | `og_catalog.schema_version`의 최댓값 |
| `entity_types[]` | array | `{name, abstract, extends[], instances, properties[]}` |
| `relation_types[]` | array | `{name, abstract, extends[], instances, roles[], properties[]}` |
| `notes[]` | array | 항상 같은 3개 문장(아래) |
| `truncated` | object | **잘렸을 때만 존재**. `{shown, total, ordered_by, hint}` |

`properties[]` 항목: `{name, type, required, key}` ([agent/mod.rs:130](../../engine/src/agent/mod.rs#L130)).
`roles[]` 항목: `{name, player_type, position, min, max}`.
`position`은 `ordinal` 0→`"source"`, 1→`"target"`, 그 외→`"additional"`
([agent/mod.rs:154](../../engine/src/agent/mod.rs#L154)).

**고정 `notes` (에이전트가 자주 틀리는 세 가지, [agent/mod.rs:95](../../engine/src/agent/mod.rs#L95))**

```
A label matches all of its subtypes: MATCH (v:Vehicle) also returns Car and EV.
Relationship direction matters. Check `roles` for which type sits at each end.
Parameters use $name and are passed as the third argument to og_cypher.
```

**결정(Decision)**: 잘린 스키마를 **잘렸다는 사실과 함께** 주는 편이, 읽을 여유가
없는 완전한 스키마를 주는 것보다 에이전트가 더 잘 행동한다
([agent/mod.rs:17](../../engine/src/agent/mod.rs#L17)).

**예제**

```sql
SELECT jsonb_pretty(og_schema('default'));
SELECT og_schema('default', 2000) -> 'truncated';
-- {"shown": 66, "total": 210, "ordered_by": "instance count, descending",
--  "hint": "call og_schema_for(graph, question) for the types relevant to one question"}
```

**실패 조건**: 그래프 없음 → `graph '<g>' does not exist`.

> ⚠️ 이 함수는 타입마다 `count(*)` 서브쿼리를 돌린다
> ([agent/mod.rs:39](../../engine/src/agent/mod.rs#L39)). 타입이 많고 데이터가 크면
> 비싸다. Studio가 매 스키마 요청마다 호출한다
> ([portal/server/index.js:172](../../portal/server/index.js#L172)).

---

### `og_schema_for(graph text, question text) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:189](../../engine/src/agent/mod.rs#L189) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 자연어 질문과 **어휘적으로** 겹치는 타입만 골라 준다(spec 008 FR-004).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `question` | `text` | 필수 | — | 자연어 질문 |

**점수 규칙 (코드 그대로, [agent/mod.rs:213](../../engine/src/agent/mod.rs#L213))**

| 조건 | 점수 |
|---|---|
| 타입 이름이 단어를 포함하거나 그 반대 | `+10` |
| 편집 거리 ≤ 2 | `+4` |
| 프로퍼티 이름이 단어를 포함하거나 그 반대 | `+3` |

- 질문은 비영숫자로 분해하고 **3글자 미만 단어는 버린다**
  ([agent/mod.rs:192](../../engine/src/agent/mod.rs#L192)).
- 점수 > 0인 타입만 남기고 내림차순 정렬 후 **상위 12개**로 자른다
  ([agent/mod.rs:234](../../engine/src/agent/mod.rs#L234)).

**반환 구조**

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `question` | text | 입력 그대로 |
| `matched_types[]` | array | `{name, kind, relevance, extends[], properties[], roles}` |
| `fallback` | text\|null | 매치가 없으면 `"no lexical match; call og_schema(graph) for the full schema"` |

`kind`는 `"relation"` 또는 `"entity"` 로만 표기된다 — **속성 타입(`a`)도
`"entity"` 로 표시된다** ([agent/mod.rs:241](../../engine/src/agent/mod.rs#L241)).

**예제**

```sql
SELECT jsonb_pretty(og_schema_for('default', 'which actors worked on animated films?'));
```

**결정(Decision)**: 영리하려 하지 않는다. 에이전트의 컨텍스트를 작게 유지하고
**라벨 이름이 실재하게** 만드는 것이 목적이다([agent/mod.rs:187](../../engine/src/agent/mod.rs#L187)).

**실패 조건**: 그래프 없음.

---

## 3. 오류 진단

### `og_explain_error(graph text, query text) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:261](../../engine/src/agent/mod.rs#L261) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 질의를 파싱·컴파일해 보고 구조화된 오류 보고서를 준다(spec 008 FR-007..FR-011). **실행하지 않는다.**

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | Cypher 질의 |

**반환 구조**

| 키 | 타입 | 언제 | 설명 |
|---|---|---|---|
| `ok` | bool | 항상 | 파싱·컴파일 성공 여부 |
| `code` | text | `ok=false` | 아래 §3.1 |
| `message` | text | `ok=false` | 원 오류 메시지 |
| `stage` | text | `ok=false` | `"parse"` \| `"compile"` |
| `suggestions` | array\|null | `stage="compile"` | 철자 후보 |

### 3.1 `code` 값과 **실제 도달 가능성**

정의: [engine/src/agent/mod.rs:294](../../engine/src/agent/mod.rs#L294) `classify()`.
분류는 오류 메시지의 **부분 문자열 매칭**으로 이루어지고, 순서대로 검사된다.

| `code` | 매칭 조건 | 실제 도달 가능? |
|---|---|---|
| `CYPHER_PARSE_ERROR` | 파서가 실패 | ✅ |
| `UNKNOWN_LABEL` | 메시지에 `"unknown label"` | ❌ **도달 불가** — 아래 참조 |
| `UNBOUND_VARIABLE` | 메시지에 `"not defined"` | ✅ `variable '<v>' is not defined in this query` |
| `UNKNOWN_FUNCTION` | 메시지에 `"unknown function"` | ✅ |
| `UNSUPPORTED_SYNTAX` | 메시지에 `"not supported"` | ❌ **Cypher 컴파일 경로에서 도달 불가** |
| `COMPILE_ERROR` | 그 외 | ✅ (실질적 기본값) |
| `INTERNAL` | 컴파일 중 패닉 | ✅ |

> ⚠️ **`UNKNOWN_LABEL`은 절대 나오지 않는다.** 존재하지 않는 라벨은 오류가 아니라
> `NOTICE`이고, 메시지는 `label '<x>' does not exist in graph '<g>' — matching
> nothing. did you mean: …` 이다([engine/src/catalog/types.rs:168](../../engine/src/catalog/types.rs#L168)).
> 컴파일은 성공하므로 `og_explain_error`는 `{"ok": true}`를 반환한다 — 라벨 오타를
> 잡으라고 만든 함수가 라벨 오타를 잡지 못한다.
>
> ⚠️ **`UNSUPPORTED_SYNTAX`도 Cypher 경로에서는 나오지 않는다.**
> `engine/src` 전체에서 `"not supported"`를 포함하는 오류는 TypeQL 쪽과
> `genai` provider 오류뿐이다(`grep -rn 'not supported' engine/src/`).
>
> `docs/cypher.md:249`은 `og_explain_error('kb', 'MATCH (a) RETURN nosuchfunction(a)')`가
> `UNSUPPORTED_SYNTAX`를 준다고 적고 있으나, `classify()`가 `"unknown function"`을
> **먼저** 검사하므로 실제 값은 `UNKNOWN_FUNCTION`이다.
>
> → [11_errors.md](11_errors.md), [12_improvements_api.md](12_improvements_api.md) **API-10**.

### 3.2 `suggestions` 생성 (코드 그대로, [agent/mod.rs:308](../../engine/src/agent/mod.rs#L308))

1. 메시지에 `"did you mean:"` 이 있으면 그 뒤를 `, `로 잘라 배열로.
2. 없고 메시지에 `"unknown label"`이 있으면 엔티티 타입 이름 20개
   (**§3.1에 따라 도달 불가 분기**).
3. 그 외 `null`.

**예제**

```sql
SELECT og_explain_error('default', 'MATCH (a) RETRUN a');
-- {"ok": false, "code": "CYPHER_PARSE_ERROR", "stage": "parse",
--  "message": "…: expected a clause keyword"}

SELECT og_explain_error('default', 'MATCH (a) RETURN nosuchfn(a)');
-- {"ok": false, "code": "UNKNOWN_FUNCTION", "stage": "compile",
--  "message": "unknown function 'nosuchfn'. supported: …", "suggestions": null}

SELECT og_explain_error('default', 'MATCH (p:Persn) RETURN p');
-- NOTICE:  label 'Persn' does not exist in graph 'default' — matching nothing. did you mean: Person
-- {"ok": true}          ← the JSON says nothing is wrong
```

**실패 조건**: 그래프 없음. 그 외에는 오류를 던지지 않는다 — 컴파일 패닉도
`catch_unwind`로 잡아 `{"code": "INTERNAL"}`로 보고한다
([agent/mod.rs:271](../../engine/src/agent/mod.rs#L271)).

---

### `og_diagnose_empty(graph text, query text) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:339](../../engine/src/agent/mod.rs#L339) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값

**무엇을 하는가**: "왜 아무것도 안 나왔는가?"에 답한다(spec 008 FR-010). 패턴을 **한 요소씩 늘려가며 실제로 실행**해서 결과가 처음 비는 지점을 보고한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | Cypher 질의. **첫 번째 `MATCH` 절만** 본다 |

**반환 구조**

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `steps[]` | array | 단계별 `{elements, description, rows}` |
| `error` | text | 파싱 실패 시 이 키만 |

마지막 단계에 판정이 붙는다 ([engine/src/cypher/mod.rs:787](../../engine/src/cypher/mod.rs#L787)):

| 상황 | `verdict` | `hint` |
|---|---|---|
| 어떤 단계에서 0행 | `this is where the match became empty` | `check the label spelling, the relationship type, and the direction of the arrow at this step` |
| 패턴은 매치했는데 `WHERE`가 다 걸러냄 | `the pattern matched; the WHERE clause removed every row` | `relax the predicate or check property names with og_schema()` |

`description`은 패턴을 다시 렌더링한 문자열이다 — 예: `(a:Person)-[:ACTED_IN]->(w:Work)`
([engine/src/cypher/mod.rs:805](../../engine/src/cypher/mod.rs#L805) `describe`).

**예제**

```sql
SELECT jsonb_pretty(og_diagnose_empty('default',
  'MATCH (p:Person)-[:DIRECTED]->(s:Series) RETURN p'));
-- {"graph": "default",
--  "steps": [{"elements": 1, "description": "(p:Person)", "rows": 8},
--            {"elements": 3, "description": "(p:Person)-[:DIRECTED]->(s:Series)", "rows": 0},
--            {"verdict": "this is where the match became empty", "hint": "…"}]}
```

**실패 조건**: 파싱 실패 시 `{"error": "<msg>"}`를 반환한다(오류를 던지지 않음).
그래프가 없으면 `graph '<g>' does not exist`로 **던진다**.

> ⚠️ 이 함수는 부분 패턴을 **실제로 실행**한다(`count(*)`). 큰 그래프에서는 비싸다.
> 그래서 `STABLE`이 아니라 기본값(`VOLATILE`)이다.

---

### `og_estimate(graph text, query text) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:350](../../engine/src/agent/mod.rs#L350) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 실행하지 않고 비용을 추정하고 구체적 조언을 준다(spec 008 FR-030, FR-031).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | **읽기** 질의 |

**반환 구조**

| 키 | 타입 | 설명 |
|---|---|---|
| `estimated_rows` | float | `EXPLAIN`의 `Plan Rows` |
| `estimated_cost` | float | `EXPLAIN`의 `Total Cost` |
| `sql` | text | 컴파일된 SQL |
| `advice[]` | array\<text\> | 아래 규칙 |
| `would_run` | bool | `advice`가 비어 있으면 `true` |
| `error` | text | 컴파일 실패 시 **이 키만** |

**조언 규칙 (코드 그대로, [agent/mod.rs:373](../../engine/src/agent/mod.rs#L373))**

| 조건 | 조언 |
|---|---|
| `rows > 1,000,000` | `estimated <n> rows — add a LIMIT or a more selective WHERE clause` |
| SQL에 `CROSS JOIN`이 있고 `LATERAL`이 없음 | `the pattern contains an unconnected node — connect it with a relationship or it becomes a cartesian product` |
| `cost > 1,000,000` | `consider an index: og_create_index(graph, type, property)` |

**예제**

```sql
SELECT og_estimate('default', 'MATCH (a:Person), (b:Person) RETURN a.name, b.name');
-- {"estimated_rows": 64, "estimated_cost": 12.5, "advice": [], "would_run": true, "sql": "…"}
```

**실패 조건**: 컴파일 실패 시 오류를 던지지 않고 `{"error": "<msg>"}` 반환.
쓰기 질의는 컴파일 자체가 실패하므로 같은 경로다.

> ⚠️ **`would_run`은 승인 신호가 아니다.** `advice`가 비었다는 뜻일 뿐이며,
> 실제로 실행을 막는 기구는 없다.

---

## 4. 가드레일 — 역할 기반 한계

### `og_create_role(name text, limits jsonb) RETURNS void`

정의: [engine/src/agent/mod.rs:404](../../engine/src/agent/mod.rs#L404) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 역할 이름에 자원 한계를 붙여 `og_catalog.agent_role`에 upsert한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `name` | `text` | 필수 | — | 역할 이름 |
| `limits` | `jsonb` | 필수 | — | 아래 키를 담은 객체. **검증되지 않는다** |

**반환**: 없음.

> ⚠️ `limits`의 스키마는 **전혀 검증되지 않는다**. 오타 난 키는 조용히 무시된다
> → [12_improvements_api.md](12_improvements_api.md) **API-21**.

### `og_apply_role(name text) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:415](../../engine/src/agent/mod.rs#L415) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 역할의 한계를 **현재 세션**에 적용한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `name` | `text` | 필수 | — | `og_create_role`로 만든 역할 이름 |

**인식되는 `limits` 키 — 그리고 실제 효과** ([agent/mod.rs:426](../../engine/src/agent/mod.rs#L426))

| 키 | 타입 | 실행되는 것 | 실제 효과 |
|---|---|---|---|
| `statement_timeout_ms` | int | `SET statement_timeout = <ms>` | ✅ 작동 |
| `work_mem_kb` | int | `SET work_mem = <n>` | ✅ 작동 |
| `read_only` | bool (`true`일 때만) | `SET default_transaction_read_only = on` | ✅ **다음 트랜잭션부터** |
| `max_rows` | int | `SET og.max_rows = <n>` | ❌ **무연산** |

> ⚠️ **`max_rows`는 아무 효과가 없다.** `og.max_rows`를 **읽는 코드가 저장소
> 어디에도 없다** (`grep -rn 'max_rows' engine/ portal/ bolt/` → `agent/mod.rs`의
> 쓰기 두 줄뿐). `docs/api.md:178`의 "row caps"는 구현되지 않은 기능이다
> → [12_improvements_api.md](12_improvements_api.md) **API-21**.

> ⚠️ 네 개의 `SET` 모두 `.ok()`로 결과를 버린다. 실패해도 반환 JSON은
> "적용됨"이라고 말한다.

**반환**

| 키 | 타입 | 설명 |
|---|---|---|
| `role` | text | 역할 이름 |
| `applied` | jsonb | ⚠️ **저장된 `limits` 원본 그대로**. 실제로 적용된 것이 아니다 |

**예제**

```sql
SELECT og_create_role('agent_readonly', '{
  "statement_timeout_ms": 5000,
  "work_mem_kb": 16384,
  "read_only": true
}'::jsonb);

SELECT og_apply_role('agent_readonly');
-- {"role": "agent_readonly", "applied": {…}}
SHOW statement_timeout;   -- 5s
```

**실패 조건**: 역할 없음 → `no agent role named '<name>'` ([agent/mod.rs:424](../../engine/src/agent/mod.rs#L424)).

---

## 5. 시간 — 히스토리와 프로버넌스

### `og_enable_history(graph text, type_name text) RETURNS void`

정의: [engine/src/agent/mod.rs:448](../../engine/src/agent/mod.rs#L448) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 타입과 **모든 서브타입**의 저장 테이블에 히스토리 캡처 트리거를 붙인다(spec 008 FR-018..FR-023).

**결정(Decision)**: 기본 꺼짐. 히스토리는 쓰기 비용이 든다([agent/mod.rs:447](../../engine/src/agent/mod.rs#L447)).

**부수 효과**
- 서브타입별로
  `CREATE OR REPLACE TRIGGER og_hist_<sub> AFTER INSERT OR UPDATE OR DELETE ON <table> FOR EACH ROW EXECUTE FUNCTION og_capture_history()`
  ([agent/mod.rs:455](../../engine/src/agent/mod.rs#L455))
- `og_catalog.setting`에 `history.<graph>.<type_name> = 'on'` 기록

트리거 함수 본체: [engine/sql/access.sql:274](../../engine/sql/access.sql#L274).
이전 버전의 `valid_to`를 `now()`로 닫고 새 행을 `og_data.og_history`에 넣는다.
`op`는 `'i'`/`'u'`/`'d'`.

**예제**

```sql
SELECT og_enable_history('default', 'Person');
```

**실패 조건**: 트리거 생성 실패 → `failed to enable history on <table>: <e>`
([agent/mod.rs:460](../../engine/src/agent/mod.rs#L460)).

> ⚠️ **끄는 함수가 없다.** `og_disable_history`는 존재하지 않는다. 트리거를 직접
> `DROP TRIGGER`로 지워야 한다 → [12_improvements_api.md](12_improvements_api.md) **API-22**.

### `og_history(id int8) RETURNS TABLE(recorded_at timestamptz, op text, payload jsonb)`

정의: [engine/src/agent/mod.rs:471](../../engine/src/agent/mod.rs#L471) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 엔티티 하나의 변경 이력을 최신순으로 반환한다(FR-022).

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `recorded_at` | `timestamptz` | 아니오 | 기록 시각. `ORDER BY recorded_at DESC` |
| `op` | `text` | 아니오 | `'i'` (insert) / `'u'` (update) / `'d'` (delete) |
| `payload` | `jsonb` | 아니오 | **물리 컬럼 이름 그대로**의 행 스냅숏 (`to_jsonb(NEW)`). 없으면 `{}` |

> ⚠️ `payload`의 키는 `p_name` 같은 **물리 컬럼 이름**이다 — `og_node_json`이
> 하는 선언 이름 매핑이 적용되지 않는다([engine/sql/access.sql:285](../../engine/sql/access.sql#L285)).

**예제**

```sql
SELECT recorded_at, op, payload FROM og_history(412316860417);
```

**실패 조건**: 없음. 히스토리가 없으면 0행.

### `og_as_of(id int8, at timestamptz) RETURNS jsonb`

정의: [engine/src/agent/mod.rs:502](../../engine/src/agent/mod.rs#L502) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 과거 시점의 엔티티 상태를 반환한다(FR-020, FR-021).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `id` | `int8` | 필수 | — | 엔티티 id |
| `at` | `timestamptz` | 필수 | — | 시점 |

**반환**: 그 시점 이하에서 가장 최근 `payload` jsonb. 해당 시점 이전 기록이 없으면 `null`.

**결정(Decision)**: 히스토리가 없으면 **현재 값을 돌려주지 않고 오류를 낸다** —
그것은 거짓말이 되기 때문:

```
ERROR:  no history is retained for entity 412316860417. enable it with
        og_enable_history(graph, type) — returning the current value instead would be a lie
```

([agent/mod.rs:512](../../engine/src/agent/mod.rs#L512))

**예제**

```sql
SELECT og_as_of(412316860417, '2026-01-01 00:00:00+09');
```

### `og_set_source(entity_id int8, source text, confidence float4 DEFAULT NULL, author text DEFAULT NULL) RETURNS void`

정의: [engine/src/agent/mod.rs:529](../../engine/src/agent/mod.rs#L529) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 엔티티에 출처 메타데이터를 붙인다(spec 008 FR-015). `og_data.og_source`에 엔티티당 한 행(upsert).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `entity_id` | `int8` | 필수 | — | 노드 또는 엣지 id |
| `source` | `text` | 필수 | — | 출처 식별자 |
| `confidence` | `float4` | 선택 | `NULL` | 신뢰도. **범위 검증 없음** |
| `author` | `text` | 선택 | `NULL` | 작성자 |

**반환**: 없음. `ingested_at`은 upsert 때 `now()`로 갱신된다.

**예제**

```sql
SELECT og_set_source(412316860417, 'wikidata:Q43416', 0.92, 'ingest-bot');
```

**실패 조건**: 쓰기 실패 시 `provenance write failed` 패닉.
읽는 전용 함수는 없다 — `og_data.og_source`를 직접 조회할 것.

---

## 6. 감사 로그

`og_cypher`와 `og_typeql`은 **성공·실패 모두** `og_data.og_audit`에 한 행을 남긴다
([engine/src/cypher/mod.rs:122](../../engine/src/cypher/mod.rs#L122),
[engine/src/typeql/mod.rs:115](../../engine/src/typeql/mod.rs#L115)).

| 컬럼 | 설명 |
|---|---|
| `audit_id`, `principal`, `at` | 부트스트랩 스키마가 채운다 |
| `query` | `[<graph>] <query text>` 형태 |
| `lang` | `'cypher'` \| `'typeql'` |
| `rows_out` | 반환 행 수 (오류 시 `0`) |
| `duration_ms` | 소요 시간 |
| `error_code` | 오류 메시지 **앞 200자**. 성공이면 `NULL` |

```sql
SELECT at, lang, rows_out, duration_ms, error_code, query
  FROM og_data.og_audit ORDER BY at DESC LIMIT 20;
```

> ⚠️ 감사 기록은 `.ok()`로 실패를 무시한다 — 로그가 유실될 수 있다.
> `og_typeql_script`는 **감사 기록을 남기지 않는다**.
> `error_code` 컬럼에 실제로 들어가는 것은 코드가 아니라 **메시지 앞 200자**다
> → [11_errors.md](11_errors.md).

---

## 7. 버전 · 설정

### `ontological_version() RETURNS text`

정의: [engine/src/lib.rs:40](../../engine/src/lib.rs#L40) · 휘발성: `IMMUTABLE` · 병렬: `PARALLEL SAFE`

`CARGO_PKG_VERSION` 을 그대로 반환한다. 현재 `engine/Cargo.toml`의 값은 `0.1.0`.
클라이언트가 기능을 게이팅할 때 쓰라고 있는 함수다.

```sql
SELECT ontological_version();   -- 0.1.0
```

### `og_set_setting(key text, value text) RETURNS void`

정의: [engine/src/compat/genai.rs:55](../../engine/src/compat/genai.rs#L55) · 휘발성: 기본값 · 병렬: 기본값

`og_catalog.setting`에 키/값을 upsert한다. **키 이름을 검증하지 않는다** —
오타는 조용히 저장된다.

알려진 키는 [09_neo4j_compat.md](09_neo4j_compat.md)의 `genai.*` 목록과
`og_enable_history`가 쓰는 `history.<graph>.<type>` 이다.

```sql
SELECT og_set_setting('genai.enabled', 'on');
```

**실패 조건**: 쓰기 실패 → `failed to set '<key>': <e>`.

---

## 8. 금지 / 필수

- **금지**: `og_explain_error`가 `{"ok": true}`를 준다고 해서 질의가 옳다고
  결론짓지 말 것. 라벨 오타는 잡히지 않는다(§3.1). 라벨 검증은
  `og_schema()` / `og_schema_for()` 로 별도로 할 것.
- **금지**: `og_apply_role`의 `max_rows`에 의존하지 말 것 — **무연산**이다.
  행 제한이 필요하면 질의에 `LIMIT`을 쓰거나 애플리케이션에서 자를 것.
- **금지**: `og_estimate`의 `would_run: true`를 실행 승인으로 쓰지 말 것.
- **금지**: `og_diagnose_empty`를 큰 그래프에 습관적으로 부르지 말 것 —
  부분 패턴을 실제로 실행한다.
- **필수**: `og_as_of`를 쓰기 전에 `og_enable_history`를 먼저 부를 것.
  그렇지 않으면 오류다(의도된 동작).
- **필수**: `og_history(...).payload`의 키가 **물리 컬럼 이름**임을 기억할 것.
  선언 이름으로 보려면 `og_property_view`로 매핑할 것.
- **필수**: 토큰 예산이 있는 에이전트는 `og_schema(graph, budget)`을 쓰고
  반환 JSON의 `truncated` 키를 반드시 확인할 것.

---

## 9. 관련 문서

- 오류 코드 전체 체계 → [11_errors.md](11_errors.md)
- Cypher 문법 경계 → [03_cypher.md](03_cypher.md)
- 원문 요약 → [docs/agents.md](../../docs/agents.md), [docs/api.md:169](../../docs/api.md)

<!-- affects: api, backend, llm -->
<!-- requires-update: 02_api/11_errors.md, 02_api/12_improvements_api.md -->
