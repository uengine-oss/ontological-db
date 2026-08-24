# 04. 실행 전 비용 추정과 검증 — `og_estimate` / `og_cypher_check`

> **이 문서가 답하는 질문**
> - `og_estimate` 는 무엇을 계산하고, 그 숫자는 어디서 오는가?
> - `advice` 배열에 들어가는 문장 세 종류는 어떤 조건에서 나오는가?
> - `would_run` 은 무엇을 보증하고, 무엇을 보증하지 않는가?
> - dry-run 계열 함수 중 무엇이 DB에 접근하고 무엇이 접근하지 않는가?

---

## 1. 사실 — dry-run 계열 함수 4종

| 함수 | DB 접근 | 쓰기 질의 지원 | 반환 | 근거 |
|---|---|---|---|---|
| `og_cypher_check(query)` | **없음** (`IMMUTABLE, PARALLEL SAFE`) | ✅ (파싱만) | `{ok, clauses, write}` 또는 `{ok:false, error}` | [engine/src/cypher/mod.rs:699-709](../../engine/src/cypher/mod.rs) |
| `og_cypher_sql(graph, query)` | 카탈로그 (`STABLE`) | ❌ | 컴파일된 SQL 문자열 | [engine/src/cypher/mod.rs:74-80](../../engine/src/cypher/mod.rs) |
| `og_cypher_explain(graph, query, analyze)` | 카탈로그 + `EXPLAIN` | ❌ | `{columns, sql, plan}` | [engine/src/cypher/mod.rs:675-696](../../engine/src/cypher/mod.rs) |
| `og_estimate(graph, query)` | 카탈로그 + `EXPLAIN` | ❌ | `{estimated_rows, estimated_cost, sql, advice, would_run}` | [engine/src/agent/mod.rs:350-397](../../engine/src/agent/mod.rs) |

`og_cypher_check` 는 파서만 돌리므로 존재하지 않는 그래프·레이블에도 `ok:true` 를
낸다. 반대로 `write: true` 를 정확히 판정하므로
([cypher/mod.rs:33-45, 705](../../engine/src/cypher/mod.rs)) **읽기/쓰기 라우팅에는 이것을
쓰는 것이 맞다.**

---

## 2. 사실 — `og_estimate` 의 실제 구현

```rust
// engine/src/agent/mod.rs:351-372
let sql = compile_for_diagnostics(graph, query)?;          // 351-355
let plan = one_mut::<JsonB>(&format!("EXPLAIN (FORMAT JSON) {sql}"),
                            &[JsonB(json!({})).into()]);   // 356-363
let root = plan[0]["Plan"];
let rows = root["Plan Rows"].as_f64().unwrap_or(0.0);      // 370
let cost = root["Total Cost"].as_f64().unwrap_or(0.0);     // 371
```

- 숫자는 **PostgreSQL 플래너의 추정치 그대로**다. 별도 모델이 없다.
- `EXPLAIN` 이지 `EXPLAIN ANALYZE` 가 아니다. 질의는 실행되지 않는다.
- spec 008 FR-030이 요구하는 "**방문 노드 수**, 예상 시간"
  ([specs/008-agent-native-interface/spec.md:271-272](../../specs/008-agent-native-interface/spec.md))
  은 반환되지 않는다. 반환되는 것은 행 수와 비용(`Total Cost`)뿐이다.

### 2.1 결함 — 파라미터가 빈 객체로 전달된다

```rust
// engine/src/agent/mod.rs:356-359
&[JsonB(json!({})).into()]
```

Cypher 파라미터는 `($1 ->> 'name')` 로 컴파일되므로
([engine/src/cypher/compile.rs:1156-1157](../../engine/src/cypher/compile.rs)),
`{}` 를 넘기면 모든 파라미터가 SQL NULL이 된다. 플래너는 그 전제로 선택도를 추정한다.

즉 **파라미터를 쓰는 질의의 `estimated_rows` 는 실제 실행 시 행 수와 다를 수 있다.**
spec 008 SC-009("dry-run 예상 행 수와 실제 행 수의 상관계수 0.8 이상",
[spec.md:312](../../specs/008-agent-native-interface/spec.md))를 파라미터 질의에 대해
검증한 하네스는 저장소에 없다.

`og_cypher_explain` 도 같은 방식으로 빈 파라미터를 쓴다
([cypher/mod.rs:683-686](../../engine/src/cypher/mod.rs)).

### 2.2 결함 — 컴파일 실패가 트랜잭션을 죽인다

```rust
// engine/src/agent/mod.rs:352-355
let sql = match crate::cypher::compile_for_diagnostics(graph, query) {
    Ok(s) => s,
    Err(e) => return JsonB(json!({ "error": e })),
};
```

`Result::Err` 는 JSON으로 반환되지만, 컴파일 경로에서 `error!` 가 발생하는 경우
(예: 존재하지 않는 graph 이름 →
[engine/src/cypher/compile.rs:155](../../engine/src/cypher/compile.rs) 의
`types::graph_id(graph)`)는 잡히지 않는다. `og_explain_error` 는
`std::panic::catch_unwind` 로 감싸지만([agent/mod.rs:271-273](../../engine/src/agent/mod.rs)),
`og_estimate` 는 감싸지 않는다.

**따라서 `og_estimate` 는 트랜잭션을 중단시킬 수 있다.** 에이전트 루프에서는
`og_estimate` 를 독립 트랜잭션에 두어야 한다.

---

## 3. 사실 — `advice` 를 생성하는 세 규칙

```rust
// engine/src/agent/mod.rs:373-388
if rows > 1_000_000.0 { advice.push("estimated {rows} rows — add a LIMIT or a more selective WHERE clause") }
if sql.matches("CROSS JOIN").count() > 0 && !sql.contains("LATERAL") {
    advice.push("the pattern contains an unconnected node — connect it with a relationship \
                 or it becomes a cartesian product")
}
if cost > 1_000_000.0 { advice.push("consider an index: og_create_index(graph, type, property)") }
```

| # | 조건 | 임계값 | 조언 |
|---|---|---|---|
| 1 | `Plan Rows` | `> 1,000,000` | LIMIT 또는 더 선택적인 WHERE |
| 2 | 컴파일된 SQL에 `CROSS JOIN` 이 있고 `LATERAL` 이 없음 | — | 연결되지 않은 노드 → 데카르트 곱 |
| 3 | `Total Cost` | `> 1,000,000` | 인덱스 생성 권고 |

**모든 임계값이 하드코딩**되어 있다. 설정(`og_catalog.setting`)이나 GUC로 조정할 수 없다.

규칙 2의 판정은 **문자열 매칭**이다. `sql.contains("LATERAL")` 은 SQL 전체에 하나라도
LATERAL이 있으면 참이므로, 가변 길이 매치(`og_vlp` 를 LATERAL로 붙이는 경로,
[engine/src/compat/procs.rs:262-267](../../engine/src/compat/procs.rs) 참조)가 섞인 질의에서는
진짜 데카르트 곱이 있어도 규칙 2가 침묵한다.

---

## 4. 사실 — `would_run` 의 의미

```rust
// engine/src/agent/mod.rs:395
"would_run": advice.is_empty(),
```

`would_run` 은 **`advice` 배열이 비었다는 사실의 재표현**일 뿐이다.

- 이 필드는 질의가 실행 가능한지, 안전한지, 권한이 있는지를 보증하지 않는다.
- `og_estimate` 는 질의를 실행하지 않으며, `would_run: false` 여도 `og_cypher` 호출을
  막지 않는다. 강제 장치가 아니다.
- `advice` 가 비어 있어도 임계값 아래의 비싼 질의는 통과한다 (예: 90만 행).

원문 [docs/agents.md:85](../../docs/agents.md) 의 표현 "Letting the agent decline its own bad
query is better than killing it later" 가 정확한 위치 설명이다 — **자발적 사양(辭讓)을
위한 정보**이지 게이트가 아니다.

### 4.1 응답 예시

```sql
SELECT og_estimate('meeting',
  $$MATCH (a:Employee), (b:Employee) RETURN a.name, b.name$$);
```

```json
{
  "estimated_rows": 25,
  "estimated_cost": 4.3,
  "sql": "SELECT jsonb_build_object(...) FROM og_data.v_12 n1 CROSS JOIN og_data.v_12 n2",
  "advice": [
    "the pattern contains an unconnected node — connect it with a relationship or it becomes a cartesian product"
  ],
  "would_run": false
}
```

`sql` 필드가 컴파일 결과 전문을 포함한다는 점에 주의 — 큰 질의에서는 응답 자체가
토큰을 크게 소비한다. 에이전트에 넘기기 전에 제거하는 것을 권장한다.

---

## 5. 결정(Decision)

| ID | 결정 | 근거 |
|---|---|---|
| D-1 | 비용 모델을 직접 만들지 않고 PostgreSQL 플래너 추정치를 그대로 노출 | [agent/mod.rs:356-371](../../engine/src/agent/mod.rs) |
| D-2 | 조언은 3개 규칙의 하드코딩. 학습·설정 없음 | [agent/mod.rs:373-388](../../engine/src/agent/mod.rs) |
| D-3 | dry-run은 **정보 제공**이며 강제 게이트가 아님 | [agent/mod.rs:395](../../engine/src/agent/mod.rs) |
| D-4 | 방문 노드 수 상한을 컴파일된 SQL에 주입하는 기능(T011)은 착수하지 않음 | [specs/008-agent-native-interface/tasks.md:22](../../specs/008-agent-native-interface/tasks.md) |

---

## 6. 필수(Required) / 금지(Forbidden)

**필수**

- 읽기/쓰기 라우팅은 `og_cypher_check(query).write` 로 판정할 것.
- `og_estimate` 는 **독립 트랜잭션**에서 호출할 것 (2.2절).
- 파라미터를 쓰는 질의의 `estimated_rows` 는 참고치로만 쓸 것 (2.1절).
- 실제 리소스 상한은 `og_apply_role` 의 `statement_timeout_ms` 로 강제할 것
  ([engine/src/agent/mod.rs:426-428](../../engine/src/agent/mod.rs)) —
  이것이 저장소에서 실제로 강제되는 유일한 상한이다
  ([08_guardrails_and_roles.md](08_guardrails_and_roles.md) 3절 참조).

**금지**

- `would_run: true` 를 안전/권한/실행 가능성 보증으로 해석 금지 (4절).
- `og_estimate` 를 쓰기 질의(`CREATE`/`MERGE`/`SET`/`DELETE`)에 호출 금지 —
  `{"error": "write queries are not compiled to a single statement"}` 만 나온다
  ([cypher/mod.rs:53-55](../../engine/src/cypher/mod.rs)).
  spec 008 FR-026("영향 범위가 임계치를 넘는 쓰기·삭제는 사전 확인 없이 거부")을
  이 함수로 만족시킬 수 없다.
- `advice` 가 비었다는 이유로 `LIMIT` 를 생략하지 말 것. 임계값은 100만 행이다.
- `sql` 필드를 그대로 LLM 컨텍스트에 넣지 말 것 (4.1절).

---

## 7. 참고

- 원문: [docs/agents.md:69-85](../../docs/agents.md)
- 함수 계약: [docs/api.md:177](../../docs/api.md)
- 스펙: FR-030/FR-031, SC-009
  ([specs/008-agent-native-interface/spec.md:269-274, 312](../../specs/008-agent-native-interface/spec.md))
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-05

<!-- affects: llm, api, backend -->
<!-- requires-update: 02_api/00_index.md -->
