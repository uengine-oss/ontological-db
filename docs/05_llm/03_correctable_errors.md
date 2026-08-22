# 03. 교정 가능한 오류 — `og_explain_error` / `og_diagnose_empty`

> **이 문서가 답하는 질문**
> - 실제로 반환되는 오류 코드는 무엇이고, 각각 어떤 메시지에서 나오는가?
> - 어떤 실패가 "교정 가능"으로 분류되고, 어떤 실패가 **오류로 잡히지도 않는가**?
> - 에이전트의 재시도 루프를 어떻게 설계해야 하는가?
> - 원문 `docs/agents.md` 의 예제와 실제 코드가 어긋나는 지점은?

---

## 1. 사실 — `og_explain_error(graph, query)` 의 실제 동작

정의: [engine/src/agent/mod.rs:261-292](../../engine/src/agent/mod.rs). `STABLE`.

두 단계로 나뉜다.

```rust
// engine/src/agent/mod.rs:263-291
match crate::cypher::parser::parse(query) {
    Err(e) => { ok:false, code:"CYPHER_PARSE_ERROR", message:e, stage:"parse" }
    Ok(_)  => match std::panic::catch_unwind(|| compile_for_diagnostics(graph, query)) {
        Ok(Ok(_))    => { ok:true }
        Ok(Err(msg)) => { ok:false, code:classify(msg), message:msg,
                          stage:"compile", suggestions:suggestions(graph,msg) }
        Err(_)       => { ok:false, code:"INTERNAL", message:"compilation aborted",
                          stage:"compile" }
    }
}
```

### 1.1 오류 코드 전체 목록

| `code` | `stage` | 생성 조건 | 근거 |
|---|---|---|---|
| `CYPHER_PARSE_ERROR` | `parse` | 파서 실패. 메시지 형식은 `"{msg} at offset {pos}, near: …{snippet}…"` | [agent/mod.rs:264-269](../../engine/src/agent/mod.rs), [cypher/parser.rs:97-101](../../engine/src/cypher/parser.rs) |
| `UNKNOWN_LABEL` | `compile` | 메시지에 `"unknown label"` 포함 | [agent/mod.rs:294-296](../../engine/src/agent/mod.rs) |
| `UNBOUND_VARIABLE` | `compile` | 메시지에 `"not defined"` 포함 | [agent/mod.rs:297-298](../../engine/src/agent/mod.rs) |
| `UNKNOWN_FUNCTION` | `compile` | 메시지에 `"unknown function"` 포함 | [agent/mod.rs:299-300](../../engine/src/agent/mod.rs) |
| `UNSUPPORTED_SYNTAX` | `compile` | 메시지에 `"not supported"` 포함 | [agent/mod.rs:301-302](../../engine/src/agent/mod.rs) |
| `COMPILE_ERROR` | `compile` | 그 외 전부 | [agent/mod.rs:303-304](../../engine/src/agent/mod.rs) |
| `INTERNAL` | `compile` | 컴파일이 패닉/ereport로 중단됨 (예: 존재하지 않는 graph 이름) | [agent/mod.rs:283-288](../../engine/src/agent/mod.rs), [cypher/compile.rs:155](../../engine/src/cypher/compile.rs) |

### 1.2 `UNKNOWN_LABEL` 은 **현재 코드에서 발생하지 않는다**

`classify()` 는 메시지에 `"unknown label"` 이 있는지 본다
([agent/mod.rs:295](../../engine/src/agent/mod.rs)). 그러나 저장소 전체에서 그 문자열을
**생성하는 코드는 없다.** 해당 문자열이 등장하는 위치는 `classify`/`suggestions` 두
곳과 주석 한 줄뿐이다
([engine/src/catalog/types.rs:203](../../engine/src/catalog/types.rs) — 주석).

실제로 Cypher 컴파일러가 알 수 없는 레이블을 만나면 **오류를 내지 않는다**:

```rust
// engine/src/catalog/types.rs:160-175 (resolve_label_set)
None => {
    // A label nothing has ever been written under is not an error —
    // in Cypher it simply matches nothing …
    let near = nearest_type_names(gid, name);
    if !near.is_empty() {
        pgrx::notice!("label '{name}' does not exist in graph '{graph}' — matching \
                       nothing. did you mean: {}", near.join(", "));
    }
    return Ok(LabelMatch::Nothing);
}
```

컴파일러는 이를 `constrain("false")` 로 바꾼다
([engine/src/cypher/compile.rs:709-715](../../engine/src/cypher/compile.rs)). 관계 타입도
같다 — `try_type_id` (Option 반환)로 조회하고 없으면 매칭 집합이 비는 것으로 끝난다
([engine/src/cypher/compile.rs:835, 918, 941](../../engine/src/cypher/compile.rs)).

**결과**: `SELECT og_explain_error('meeting', 'MATCH (p:Emploee) RETURN p')` 는
`{"ok": true}` 를 반환한다. 교정 정보는 PostgreSQL **NOTICE** 로만 나가며, 이는
`og_explain_error` 의 JSON 응답에 들어가지 않는다.

**결정(Decision)**: 오타 레이블을 오류로 만들지 않는 것은 의도된 설계다 — Neo4j 호환을
위해서다([types.rs:161-165](../../engine/src/catalog/types.rs) 주석). 대가는 오타가
`og_explain_error` 로 잡히지 않고 `og_diagnose_empty` 단계까지 밀린다는 것.

### 1.3 `suggestions()` 의 실제 동작

```rust
// engine/src/agent/mod.rs:308-332
if let Some(rest) = msg.split("did you mean:").nth(1) {
    return rest.trim().trim_end_matches('.').split(", ").collect();
}
if msg.contains("unknown label") { /* 이 분기는 도달 불가 — 1.2절 */ }
Value::Null
```

동작하는 것은 **첫 번째 분기뿐**이다. `"did you mean:"` 을 실어 나르는 메시지는
`types::type_id` 와 TypeQL 경로에서 나온다:

| 메시지 생성 위치 | 예시 |
|---|---|
| [engine/src/catalog/types.rs:135](../../engine/src/catalog/types.rs) | `type 'Emploee' does not exist. did you mean: Employee` |
| [engine/src/typeql/compile.rs:126](../../engine/src/typeql/compile.rs) | `type 'x' is not defined. did you mean: …` |
| [engine/src/typeql/write.rs:185](../../engine/src/typeql/write.rs) | 동일 |

이 중 `types::type_id` 는 **Cypher 읽기 경로가 아니라** DDL·함수 인자 경로에서 쓰인다
(`og_add_property`, `og_add_embedding`, `og_vector_search`, `og_enable_history` 등).
그리고 `type_id` 는 `Result` 가 아니라 `error!` 를 던지므로
([types.rs:129-137](../../engine/src/catalog/types.rs)), `og_explain_error` 로는 코드
`INTERNAL` 로만 도달한다.

### 1.4 후보 생성 규칙 — `nearest_type_names`

```rust
// engine/src/catalog/types.rs:236-261
let target = name.to_ascii_lowercase();
// 그래프의 모든 타입 이름과 Levenshtein 거리를 계산
let cutoff = (target.chars().count() / 2).max(2);
scored.filter(|(d,_)| *d <= cutoff).take(3)
```

| 입력 이름 길이 | `cutoff` (허용 편집거리) |
|---|---|
| 1~5 글자 | 2 |
| 6~7 글자 | 3 |
| 8~9 글자 | 4 |
| 20 글자 | 10 |

- 최대 **3개** 후보.
- 편집거리는 Rust 구현(`edit_distance`, 2행 Levenshtein,
  [types.rs:264-284](../../engine/src/catalog/types.rs)) — `fuzzystrmatch` 확장에 의존하지 않는다.
- 비교는 소문자 접기 후 수행하되 **ASCII만** 접힌다.
- 프로퍼티 이름에 대한 후보 제안은 없다 (spec 008 FR-008이 요구하는 "프로퍼티 후보 목록"
  미구현. [specs/008-agent-native-interface/spec.md:69-70](../../specs/008-agent-native-interface/spec.md)).

### 1.5 실제로 `COMPILE_ERROR` / `UNBOUND_VARIABLE` / `UNKNOWN_FUNCTION` 을 내는 메시지

| 메시지 | 결과 코드 | 근거 |
|---|---|---|
| `variable 'x' is not defined in this query` | `UNBOUND_VARIABLE` | [compile.rs:1091, 1190](../../engine/src/cypher/compile.rs) |
| `unknown function 'foo'. supported: count, sum, …` | `UNKNOWN_FUNCTION` | [compile.rs:1558-1565](../../engine/src/cypher/compile.rs) |
| `property access is only supported on pattern variables` | `COMPILE_ERROR` | [compile.rs:1178](../../engine/src/cypher/compile.rs) |
| `'p' is a path; use length(p) or nodes(p)` | `COMPILE_ERROR` | [compile.rs:1188](../../engine/src/cypher/compile.rs) |
| `id() expects a pattern variable` | `COMPILE_ERROR` | [compile.rs:1451](../../engine/src/cypher/compile.rs) |
| `this clause is only valid in a write query` | `COMPILE_ERROR` | [compile.rs:364](../../engine/src/cypher/compile.rs) |
| `genai.vector.encode(...) needs the text to encode` | `COMPILE_ERROR` | [compile.rs:1547-1549](../../engine/src/cypher/compile.rs) |
| 쓰기 질의 (`CREATE`/`MERGE`/`SET`/…) | `COMPILE_ERROR` — 메시지: `write queries are not compiled to a single statement` | [cypher/mod.rs:53-55](../../engine/src/cypher/mod.rs) |

**중요**: 마지막 항목 때문에 `og_explain_error` 는 **쓰기 질의를 검증할 수 없다.**
`CREATE`/`MERGE`/`SET`/`DELETE` 를 넘기면 항상 `COMPILE_ERROR` 가 나온다.
쓰기 질의의 사전 검증은 `og_cypher_check` (파싱만) 뿐이다
([cypher/mod.rs:699-709](../../engine/src/cypher/mod.rs)).

---

## 2. 사실 — `og_diagnose_empty(graph, query)`

정의: [engine/src/agent/mod.rs:339-347](../../engine/src/agent/mod.rs) →
[engine/src/cypher/mod.rs:747-803](../../engine/src/cypher/mod.rs).

### 2.1 알고리즘

첫 번째 `MATCH` 절의 패턴을 왼쪽부터 **한 요소씩 늘려가며** `count(*)` 를 실행하고,
처음으로 0이 되는 지점을 보고한다.

```rust
// engine/src/cypher/mod.rs:755-793
for pattern in patterns {
    for elem in &pattern.elems {
        upto.push(elem.clone());
        if matches!(elem, PatElem::Rel(_)) { continue; }   // 관계에서 끊지 않음
        // (upto)까지의 부분 패턴을 컴파일해 count(*) 실행
        if n == 0 { steps.push({ verdict:"this is where the match became empty", … }); return }
    }
}
if where_.is_some() {
    steps.push({ verdict:"the pattern matched; the WHERE clause removed every row", … })
}
```

### 2.2 응답 형태

```json
{ "graph": "meeting",
  "steps": [
    { "elements": 1, "description": "(e:Employee)", "rows": 5 },
    { "elements": 3, "description": "(e:Employee)<-[:RESERVED_BY]-(r:Reservation)", "rows": 8 },
    { "verdict": "the pattern matched; the WHERE clause removed every row",
      "hint": "relax the predicate or check property names with og_schema()" }
  ] }
```

`description` 은 AST에서 재구성한 문자열이고, 방향 표기는 `-[:T]->` / `<-[:T]-` / `-[:T]-`
로 복원된다([cypher/mod.rs:805-823](../../engine/src/cypher/mod.rs)).

### 2.3 결함 — 파라미터가 전달되지 않는다

```rust
// engine/src/cypher/mod.rs:776
let n = exec_json(&compiled.sql, &json!({}))
```

부분 패턴은 **빈 파라미터**로 실행된다. Cypher 파라미터는 컴파일 시
`($1 ->> 'name')` 형태가 되므로([compile.rs:1156-1157](../../engine/src/cypher/compile.rs)),
`MATCH (m:MeetingRoom {name: $room})` 같은 인라인 프로퍼티 맵을 가진 패턴은
**항상 0행**이 되어 "this is where the match became empty" 를 오보한다.

또한 `WHERE` 절은 부분 패턴 컴파일에 포함되지 않는다
([cypher/mod.rs:763-775](../../engine/src/cypher/mod.rs) — `compile_pattern` + `count(*)` 프로젝션만).
따라서 `rows` 수치는 WHERE 이전 값이다.

### 2.4 미구현 — 관계 방향 진단

spec 008 FR-009는 "관계 방향 오류는 빈 결과가 아니라 **'반대 방향에 N건 존재'** 정보와
함께 제공되어야 한다"고 요구한다
([specs/008-agent-native-interface/spec.md:228-229](../../specs/008-agent-native-interface/spec.md)).
구현된 힌트는 고정 문자열이다
([cypher/mod.rs:786-790](../../engine/src/cypher/mod.rs)):

> "check the label spelling, the relationship type, and the direction of the arrow at this step"

반대 방향의 실제 행 수를 세는 코드는 없다.

---

## 3. 결정(Decision) — 에이전트 재시도 루프 설계 가이드

DB가 제공하는 신호만으로 루프를 짜면 다음이 된다. 각 분기의 근거는 위 절들이다.

```
질문
 └─ og_schema_for(graph, question)          [스키마 확보]
     └─ (LLM) Cypher 생성
         ├─ og_cypher_check(query)          [구문만, DB 접근 없음, 쓰기도 검증됨]
         │   └─ ok=false → CYPHER_PARSE_ERROR 로 재작성 (재시도 1)
         ├─ og_explain_error(graph, query)  [읽기 질의만 의미 있음]
         │   ├─ code=UNBOUND_VARIABLE  → 변수 바인딩 수정 (재시도 1)
         │   ├─ code=UNKNOWN_FUNCTION  → message의 supported 목록으로 치환 (재시도 1)
         │   ├─ code=COMPILE_ERROR + suggestions≠null → 후보로 치환 (재시도 1)
         │   ├─ code=COMPILE_ERROR + suggestions=null → 전면 재작성 (재시도 1)
         │   └─ code=INTERNAL → graph 이름/권한 확인. 재시도해도 같은 결과
         ├─ og_estimate(graph, query)       [비용]
         │   └─ advice≠[] → 질의 좁히기 (재시도 1)
         └─ og_cypher(graph, query, params)
             ├─ rows>0  → 답변 생성
             ├─ rows=0  → [새 트랜잭션] og_diagnose_empty
             │             └─ verdict 위치의 요소를 재작성 (재시도 1)
             └─ ERROR   → [새 트랜잭션] og_explain_error 로 되돌아감
```

**필수 설계 규칙**

| 규칙 | 이유 |
|---|---|
| 재시도 상한을 **에이전트 쪽에서** 강제할 것 | 반복 실패 질의 속도 제한(FR-029 / T012)은 미구현 ([tasks.md:23](../../specs/008-agent-native-interface/tasks.md)) |
| 진단 함수는 **새 트랜잭션**에서 호출 | `og_cypher` 실패 시 트랜잭션이 중단됨 ([cypher/mod.rs:96-98, 140](../../engine/src/cypher/mod.rs)) |
| 0행일 때 `og_diagnose_empty` 를 호출하되, 파라미터를 쓰는 질의라면 **결과를 의심**할 것 | 2.3절 |
| `{"ok": true}` 를 받았어도 0행이면 레이블 오타를 다시 의심할 것 | 1.2절 |
| 서버 로그/NOTICE 채널을 캡처할 것 | 레이블 오타 후보는 NOTICE로만 나간다 (1.2절) |
| 쓰기 질의는 `og_cypher_check` + `og_estimate` 로 검증할 수 없음을 전제할 것 | 1.5절 마지막 행, [agent/mod.rs:352-354](../../engine/src/agent/mod.rs) |

Bolt 클라이언트(Neo4j 드라이버, MCP 서버)를 쓰는 경우 NOTICE는 드라이버의 notification
채널로 전달되는지 **미확인**이다. 저장소에 그 경로를 확인하는 테스트는 없다.

---

## 4. 사실 — 원문 문서와 코드의 불일치

[docs/agents.md:87-105](../../docs/agents.md) 는 다음 예제를 싣는다.

```sql
SELECT og_explain_error('default', 'MATCH (p:Persn) RETURN p');
```
```json
{ "ok": false, "code": "UNKNOWN_LABEL", "stage": "compile",
  "message": "unknown label 'Persn' in graph 'default'. did you mean: Person",
  "suggestions": ["Person"] }
```

**이 출력은 현재 코드에서 나오지 않는다.** 근거는 1.2절이다. 실제 반환값은
`{"ok": true}` 이며, `"unknown label 'Persn' in graph 'default'"` 라는 문자열을
생성하는 코드는 저장소에 없다.

동일한 취지의 회귀 테스트가 존재하지만, 기대 출력이 저장소에 없어 무엇이 정답으로
간주되는지 확인할 수 없다
([engine/tests/sql/03_vector_agent_rdf.sql:41-42](../../engine/tests/sql/03_vector_agent_rdf.sql) —
`og_explain_error('kb', 'MATCH (d:Docs) RETURN d')`; `engine/tests/expected/` 디렉터리 부재).

**이 문서는 코드를 정답으로 삼는다.** 원문 수정은 이 카테고리의 범위 밖이며,
개선 항목 [10_improvements_llm.md](10_improvements_llm.md) LLM-03 으로 등록한다.

---

## 5. 필수(Required) / 금지(Forbidden)

**필수**

- 재시도 분기는 `code` 로 할 것. `message` 문자열 매칭 금지 (FR-011의 안정성 보장 대상은 `code`).
- `og_explain_error` 호출 시 `graph` 이름을 검증할 것. 없는 그래프는 `INTERNAL` 로 나온다.
- `suggestions` 가 `null` 인 경우와 `[]` 인 경우를 구분할 것. `null` 은 "제안 없음",
  빈 배열은 `"did you mean:"` 뒤가 비었던 경우다.

**금지**

- `og_explain_error` 의 `{"ok": true}` → "질의가 안전하다/결과가 있다"로 해석 금지.
- `og_diagnose_empty` 를 파라미터 바인딩이 필요한 질의에 쓰고 그 결과를 신뢰하는 것 금지 (2.3절).
- `og_explain_error` 를 쓰기 질의에 사용하는 것 금지 — 항상 실패한다 (1.5절).
- `std::panic::catch_unwind` 로 잡힌 `INTERNAL` 이후에 같은 세션에서 계속 질의하는 것을
  전제하지 말 것. pgrx는 이 목적에 `PgTryBuilder` 를 두고 있으며, 원시 `catch_unwind` 로
  ereport를 삼킨 뒤의 백엔드 상태는 이 저장소에서 검증되지 않았다(미확인).
  근거 위치: [engine/src/agent/mod.rs:271-273](../../engine/src/agent/mod.rs),
  [engine/Cargo.toml:39-43](../../engine/Cargo.toml) (`panic = "unwind"`).

---

## 6. 참고

- 원문: [docs/agents.md:87-124](../../docs/agents.md)
- 함수 계약: [docs/api.md:175-176](../../docs/api.md)
- 스펙: FR-007~FR-011 ([specs/008-agent-native-interface/spec.md:222-233](../../specs/008-agent-native-interface/spec.md))
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-03, LLM-04

<!-- affects: llm, api, backend -->
<!-- requires-update: 02_api/00_index.md, 05_llm/01_agent_native_interface.md -->
