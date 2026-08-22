# 오류 체계 — 코드 · 메시지 · 계층별 매핑

> **이 문서가 답하는 질문**
> - 이 확장의 오류에 SQLSTATE가 있는가? (요약: **사실상 없다**)
> - `error!` / `.expect()` / `.unwrap()`이 사용자에게 어떻게 보이는가?
> - `og_explain_error`가 실제로 낼 수 있는 코드는?
> - 같은 오류가 SQL / Bolt / Studio에서 각각 어떻게 보이는가?
> - 어떤 실패가 **오류가 아니라 NOTICE 또는 침묵**인가?

---

## 1. 사실 — SQLSTATE 규율이 없다

`engine/src/` 전체에서 `ERRCODE`, `PgSqlErrorCode`, `ereport`, `sqlstate` 를 쓰는
코드는 **한 줄도 없다** (`grep -rn 'ERRCODE\|PgSqlErrorCode\|ereport\|sqlstate' engine/src/`
→ [engine/src/id.rs:31](../../engine/src/id.rs#L31)의 주석 한 줄만 매치).

모든 사용자 대면 오류는 두 경로 중 하나로 나온다.

| 경로 | 개수 | SQLSTATE |
|---|---|---|
| pgrx `error!("…")` 매크로 | 115 (모듈별 집계는 §2) | pgrx 기본값 — **`XX000` (internal_error)** |
| `.expect("…")` / `.unwrap()` 패닉 | `.expect` 111, `.unwrap()` 75 | pgrx가 패닉을 `ERROR`로 변환 — 동일하게 `XX000` |

**귀결**: 클라이언트는 SQLSTATE로 오류를 분류할 수 없다.
"라벨 오타"와 "SPI 내부 실패"가 같은 코드다.
→ [12_improvements_api.md](12_improvements_api.md) **API-31**.

이 공백을 메우기 위해 **세 곳에서 각자 문자열 매칭**을 한다:

| 계층 | 함수 | 위치 |
|---|---|---|
| 엔진 | `agent::classify()` | [engine/src/agent/mod.rs:294](../../engine/src/agent/mod.rs#L294) |
| Bolt | `Failure::from_pg()` | [bolt/src/session.rs:578](../../bolt/src/session.rs#L578) |
| Studio | `pgError()` (분류하지 않고 그대로 전달) | [portal/server/index.js:67](../../portal/server/index.js#L67) |

---

## 2. 오류 매크로 사용 분포 (실측)

`grep -rc 'error!(' engine/src/`

| 파일 | `error!` 호출 수 |
|---|---|
| `engine/src/cypher/mod.rs` | 19 |
| `engine/src/catalog/types.rs` | 18 |
| `engine/src/interop/mod.rs` | 12 |
| `engine/src/compat/ddl.rs` | 12 |
| `engine/src/storage/mod.rs` | 11 |
| `engine/src/vector/mod.rs` | 10 |
| `engine/src/compat/genai.rs` | 8 |
| `engine/src/typeql/mod.rs` | 7 |
| `engine/src/storage/traverse.rs` | 6 |
| `engine/src/id.rs` | 3 |
| `engine/src/catalog/labeling.rs` | 3 |
| `engine/src/agent/mod.rs` | 3 |
| `engine/src/adapters/rdf.rs` | 2 |
| `engine/src/cypher/views.rs` | 1 |

`panic!()`은 **테스트 모듈에만** 존재한다 (`engine/src/cypher/parser.rs:1146`,
`engine/src/typeql/parser.rs:982`, `engine/src/adapters/rdf.rs:880` — 모두
`#[cfg(test)]` 안). 프로덕션 경로의 패닉은 전부 `.expect()` / `.unwrap()` 이다.

---

## 3. 오류 메시지 계층

### 3.1 A급 — 사람이 읽고 고칠 수 있는 메시지

교정 정보를 담고 있으며, 이 프로젝트가 잘하는 부분이다.

| 메시지 | 위치 |
|---|---|
| `type '<t>' does not exist. did you mean: Person, Persona` | [catalog/types.rs:135](../../engine/src/catalog/types.rs#L135) |
| `role '<r>' of relation '<R>' requires a '<expected>', got '<got>'` | [storage/mod.rs:480](../../engine/src/storage/mod.rs#L480) |
| `cannot add required property '<p>' to '<T>': <n> existing instance(s) would violate it. add it as optional, backfill, then tighten.` | [catalog/types.rs:532](../../engine/src/catalog/types.rs#L532) |
| `type '<t>' has <n> instance(s) (including subtypes). pass cascade => true to remove them` | [catalog/types.rs:698](../../engine/src/catalog/types.rs#L698) |
| `unknown function '<f>'. supported: count, sum, avg, …` | [cypher/compile.rs:1559](../../engine/src/cypher/compile.rs#L1559) |
| `procedure '<p>' is not available. supported: db.index.vector.queryNodes, …` | [compat/procs.rs:154](../../engine/src/compat/procs.rs#L154) |
| `there is no index named '<n>'. known indexes: room_name, person_name` | [compat/procs.rs:275](../../engine/src/compat/procs.rs#L275) |
| `unsupported property type '<t>'. supported: string, int, long, …` | [catalog/types.rs:38](../../engine/src/catalog/types.rs#L38) |
| `no history is retained for entity <id>. enable it with og_enable_history(graph, type) — returning the current value instead would be a lie` | [agent/mod.rs:512](../../engine/src/agent/mod.rs#L512) |
| `genai.vector.encode is disabled. … SELECT og_set_setting('genai.enabled', 'on')` | [compat/genai.rs:102](../../engine/src/compat/genai.rs#L102) |
| `'has $a' without an attribute type is not supported: name the attribute type` | [typeql/compile.rs:319](../../engine/src/typeql/compile.rs#L319) |
| `'<kw>' is TypeDB 2.x syntax. this engine implements TypeQL 3.x — use 'select'/'fetch' instead of 'get'` | [typeql/parser.rs:184](../../engine/src/typeql/parser.rs#L184) |

### 3.2 B급 — 원인은 알려주지만 교정은 없는 메시지

| 메시지 | 위치 |
|---|---|
| `cypher parse error: <pos>: expected a clause keyword` | [cypher/mod.rs:97](../../engine/src/cypher/mod.rs#L97) |
| `cypher error: variable '<v>' is not defined in this query` | [cypher/compile.rs:1091](../../engine/src/cypher/compile.rs#L1091) |
| `direction must be 'o', 'i' or 'b', not '<c>'` | [storage/traverse.rs:39](../../engine/src/storage/traverse.rs#L39) |
| `graph '<g>' does not exist` | [catalog/types.rs:118](../../engine/src/catalog/types.rs#L118) |
| `'<T>' is not an entity type` / `'<R>' is not a relation type` | [storage/mod.rs:255](../../engine/src/storage/mod.rs#L255), [:411](../../engine/src/storage/mod.rs#L411) |
| `no compiled graph in this backend — call og_csr_build() first` | [storage/traverse.rs:340](../../engine/src/storage/traverse.rs#L340) |
| `no embedding named '<p>' is declared on this type` | [vector/mod.rs:86](../../engine/src/vector/mod.rs#L86) |

### 3.3 C급 — 내부 메시지가 사용자에게 그대로 노출되는 경우

`.expect("…")` 문자열이 그대로 `ERROR` 메시지가 된다. 사용자 관점에서 아무
의미가 없다.

| 사용자가 보는 것 | 실제 원인 | 위치 예시 |
|---|---|---|
| `property insert failed` | 프로퍼티 재선언 (유일 제약 위반) | [catalog/types.rs:545](../../engine/src/catalog/types.rs#L545) |
| `index creation failed` | 존재하지 않는 컬럼에 인덱스 시도 | [catalog/types.rs:611](../../engine/src/catalog/types.rs#L611) |
| `update failed` | 캐스트 실패 또는 제약 위반 | [storage/mod.rs:324](../../engine/src/storage/mod.rs#L324) |
| `graph lookup failed` / `type lookup failed` | SPI 실패 | [catalog/types.rs:117](../../engine/src/catalog/types.rs#L117) |
| `id allocation failed` | 시퀀스/할당 테이블 문제 | [storage/mod.rs:31](../../engine/src/storage/mod.rs#L31) |
| `node registry insert failed` | 레지스트리 INSERT 실패 | [storage/mod.rs:275](../../engine/src/storage/mod.rs#L275) |
| `repack failed` | 재구성 SQL 실패 | [storage/stats.rs:162](../../engine/src/storage/stats.rs#L162) |

가장 자주 만나게 되는 것: **같은 프로퍼티를 두 번 선언하면 `property insert failed`**.
"이미 선언되어 있다"는 정보가 없다 → [12_improvements_api.md](12_improvements_api.md) **API-31**.

### 3.4 D급 — 컴파일된 SQL이 통째로 노출되는 경우

```
ERROR:  cypher execution failed: <postgres error>
--- compiled SQL ---
SELECT to_jsonb(...) FROM og_data.n_12 AS n1 CROSS JOIN LATERAL ...
```

([cypher/mod.rs:149](../../engine/src/cypher/mod.rs#L149))

디버깅에는 훌륭하지만 **내부 스키마(테이블 이름, 컬럼 이름, 타입 id)를 전부
노출**한다. Studio가 인증 없이 이걸 그대로 클라이언트에 전달한다
→ [12_improvements_api.md](12_improvements_api.md) **API-31**.

---

## 4. `og_explain_error`의 코드 체계

정의: [engine/src/agent/mod.rs:261](../../engine/src/agent/mod.rs#L261),
분류: [engine/src/agent/mod.rs:294](../../engine/src/agent/mod.rs#L294).

분류는 오류 **메시지 문자열의 부분 일치**이고, **순서대로** 검사된다.

```rust
if msg.contains("unknown label")      { "UNKNOWN_LABEL" }
else if msg.contains("not defined")   { "UNBOUND_VARIABLE" }
else if msg.contains("unknown function") { "UNKNOWN_FUNCTION" }
else if msg.contains("not supported") { "UNSUPPORTED_SYNTAX" }
else                                  { "COMPILE_ERROR" }
```

### 4.1 코드별 도달 가능성 (검증됨)

| 코드 | `stage` | 트리거 | 도달 가능? |
|---|---|---|---|
| `CYPHER_PARSE_ERROR` | `parse` | 파서 실패 | ✅ |
| `UNBOUND_VARIABLE` | `compile` | `variable '<v>' is not defined in this query` | ✅ |
| `UNKNOWN_FUNCTION` | `compile` | `unknown function '<f>'. supported: …` | ✅ |
| `COMPILE_ERROR` | `compile` | 그 외 컴파일 오류 전부 | ✅ (실질적 기본값) |
| `INTERNAL` | `compile` | 컴파일 중 패닉 (`catch_unwind`) | ✅ |
| `UNKNOWN_LABEL` | — | 메시지에 `"unknown label"` | ❌ **도달 불가** |
| `UNSUPPORTED_SYNTAX` | — | 메시지에 `"not supported"` | ❌ **Cypher 경로에서 도달 불가** |

### 4.2 `UNKNOWN_LABEL`이 도달 불가한 이유

`grep -rn 'unknown label' engine/src/` 결과는 세 곳뿐이고 **아무도 오류를 내지 않는다**:
- [engine/src/agent/mod.rs:295](../../engine/src/agent/mod.rs#L295) — `classify()`의 검사 자체
- [engine/src/agent/mod.rs:317](../../engine/src/agent/mod.rs#L317) — `suggestions()`의 검사 자체
- [engine/src/catalog/types.rs:203](../../engine/src/catalog/types.rs#L203) — **주석**

실제 동작: 존재하지 않는 라벨은 **`NOTICE`** 를 내고 "아무것도 매치하지 않음"으로
컴파일된다([engine/src/catalog/types.rs:168](../../engine/src/catalog/types.rs#L168)):

```
NOTICE:  label 'Persn' does not exist in graph 'default' — matching nothing. did you mean: Person
```

**결정(Decision)**: 이것은 의도된 설계다 — Cypher에서 존재하지 않는 라벨은 오류가
아니고, 라벨을 만들기 전에 탐색하는 호출자가 이 동작에 의존한다
([catalog/types.rs:161](../../engine/src/catalog/types.rs#L161)).

**하지만** 그 결과 `og_explain_error`는 **라벨 오타에 `{"ok": true}`를 반환**한다.
라벨 오타를 잡으라고 만든 함수가 그것을 잡지 못한다.

### 4.3 `UNSUPPORTED_SYNTAX`가 도달 불가한 이유

`grep -rn 'not supported' engine/src/` 결과:

| 위치 | 문맥 |
|---|---|
| `engine/src/agent/mod.rs:301` | `classify()`의 검사 자체 |
| `engine/src/compat/genai.rs:123` | `provider '<p>' is not supported` — Cypher 컴파일이 아님 |
| `engine/src/typeql/compile.rs:319` | TypeQL |
| `engine/src/typeql/mod.rs:501` | TypeQL |
| `engine/src/typeql/parser.rs:179` | TypeQL |

Cypher 컴파일러가 내는 오류 중 `"not supported"`를 포함하는 것은 없다.
`compile.rs:1178`의 `property access is only supported on pattern variables`는
`"only supported"`이지 `"not supported"`가 아니다.

### 4.4 ⚠️ 원문 문서와의 불일치

[docs/cypher.md:249](../../docs/cypher.md)은 이렇게 적고 있다:

```sql
SELECT og_explain_error('kb', 'MATCH (a) RETURN nosuchfunction(a)');
-- {"ok": false, "code": "UNSUPPORTED_SYNTAX", "message": "unknown function …"}
```

`classify()`가 `"unknown function"`을 **먼저** 검사하므로 실제 값은
`"UNKNOWN_FUNCTION"` 이다. 문서 예제가 틀렸다.

[docs/cypher.md:295](../../docs/cypher.md)은 이렇게 적고 있다:

```
ERROR:  unknown label 'Persn' in graph 'social'. did you mean: Person
```

**코드에 그런 `ERROR`가 존재하지 않는다.** 실제로는 위 §4.2의 `NOTICE`다.

→ [12_improvements_api.md](12_improvements_api.md) **API-10**.

---

## 5. 계층별 매핑 — 같은 오류가 어떻게 보이는가

예: `MATCH (p:Person) RETURN nosuchfn(p)`

| 계층 | 결과 |
|---|---|
| **SQL (`og_cypher`)** | `ERROR: cypher error: unknown function 'nosuchfn'. supported: …` <br> SQLSTATE `XX000` |
| **`og_explain_error`** | `{"ok": false, "code": "UNKNOWN_FUNCTION", "stage": "compile", "message": "unknown function 'nosuchfn'. supported: …", "suggestions": null}` |
| **Bolt** | 메시지에 `"not supported"`/`"expected"`/`"unknown label"`/`"is not defined"` 중 어느 것도 없으므로 → `Neo.ClientError.Statement.ArgumentError` <br> ([bolt/src/session.rs:578](../../bolt/src/session.rs#L578)) |
| **Studio `POST /api/cypher`** | `400` + `{"error": "cypher error: unknown function …", "code": "XX000", "detail": null, "hint": null, "where": null}` |

### 5.1 Bolt의 Neo4j 코드 매핑 (문자열 기반)

정의: [bolt/src/session.rs:578](../../bolt/src/session.rs#L578)

| 메시지에 포함 | Neo4j 코드 |
|---|---|
| `not supported` \| `expected` \| `unknown label` \| `is not defined` | `Neo.ClientError.Statement.SyntaxError` |
| `does not exist` | `Neo.ClientError.Database.DatabaseNotFound` |
| `permission denied` | `Neo.ClientError.Security.Forbidden` |
| 그 외 | `Neo.ClientError.Statement.ArgumentError` |

별도 경로:

| 상황 | 코드 |
|---|---|
| `og_cypher_check`가 `ok: false` | `Neo.ClientError.Statement.SyntaxError` ([session.rs:455](../../bolt/src/session.rs#L455)) |
| `HELLO` 전 메시지 / PostgreSQL 연결 실패 | `Neo.ClientError.Security.Unauthorized` |
| 알 수 없는 Bolt 메시지 | `Neo.ClientError.Request.Invalid` |
| 열린 결과 없는 `PULL` | `Neo.ClientError.Request.Invalid` |

**문제점 (확인됨)**
- `graph 'x' does not exist`(그래프 없음)와 `type 'x' does not exist`(타입 없음)가
  **둘 다 `DatabaseNotFound`** 가 된다. 후자는 데이터베이스 문제가 아니다.
- `"expected"`는 매우 흔한 단어라 파서와 무관한 오류도 `SyntaxError`가 될 수 있다.
- `"unknown label"` 분기는 §4.2에 따라 **영원히 매치되지 않는다.**

---

## 6. ⚠️ 오류가 아닌 실패 — 침묵과 NOTICE

계약 관점에서 가장 위험한 부분이다. 아래는 **호출자가 감지할 수 없는** 실패들이다.

| 상황 | 실제 동작 | 근거 |
|---|---|---|
| Cypher `UNION` | **조용히 첫 절반만 반환** | `compile.rs`에 `union` 참조 0건 ([03_cypher.md §4.1](03_cypher.md)) |
| 존재하지 않는 라벨 | `NOTICE` + 0행 | [catalog/types.rs:168](../../engine/src/catalog/types.rs#L168) |
| 한 상속 체인에 없는 다중 라벨 | 조용히 `LabelMatch::Nothing` → 0행 | [catalog/types.rs:187](../../engine/src/catalog/types.rs#L187) |
| 존재하지 않는 id에 `og_set_node_props` | 0행 UPDATE, 성공 반환 | [storage/mod.rs:320](../../engine/src/storage/mod.rs#L320) |
| 존재하지 않는 id에 `og_delete_node` | **`1` 반환** (`og_delete_edge`는 `0`) | [storage/mod.rs:355](../../engine/src/storage/mod.rs#L355) |
| `source_prop` 없는 임베딩에 `og_mark_embedded` | 조용히 `return` | [vector/mod.rs:368](../../engine/src/vector/mod.rs#L368) |
| `og_apply_role`의 `max_rows` | `SET og.max_rows`만 하고 **읽는 코드 없음** | [agent/mod.rs:437](../../engine/src/agent/mod.rs#L437) |
| `og_apply_role`의 `SET` 실패 | `.ok()`로 무시, JSON은 "적용됨" | [agent/mod.rs:427](../../engine/src/agent/mod.rs#L427) |
| `og_typeql`의 `_params` | **완전히 무시** | [typeql/mod.rs:52](../../engine/src/typeql/mod.rs#L52) |
| `og_similar`의 `graph` | **완전히 무시** (`let _ = graph;`) | [vector/mod.rs:173](../../engine/src/vector/mod.rs#L173) |
| `og_dump_rdf`의 잘못된 `format` | 조용히 Turtle | [adapters/rdf.rs:697](../../engine/src/adapters/rdf.rs#L697) |
| `apoc.neighbors.tohop`의 타입 필터 | 무시 (`NULL::int4[]` 전달) | [compat/procs.rs:264](../../engine/src/compat/procs.rs#L264) |
| Bolt의 시간/공간 구조체 | `"<unsupported struct 0x..>"` **문자열로 대체** | [bolt/src/session.rs:559](../../bolt/src/session.rs#L559) |
| `og_stale_embeddings`의 개별 타입 스캔 실패 | `.unwrap_or_default()` → 결과에서 누락 | [vector/mod.rs:347](../../engine/src/vector/mod.rs#L347) |
| `og_materialize_mapping`의 PK 추가 / 레지스트리 등록 실패 | `.ok()`로 무시, 성공 반환 | [interop/mod.rs:144](../../engine/src/interop/mod.rs#L144) |
| 감사 로그 기록 실패 | `.ok()`로 무시 | [cypher/mod.rs:122](../../engine/src/cypher/mod.rs#L122) |
| `og_typeql_script`의 감사 | **아예 기록하지 않음** | [typeql/mod.rs:99](../../engine/src/typeql/mod.rs#L99) |

**필수 규칙**: 위 표의 동작을 성공으로 해석하지 말 것.
특히 `UNION`, `og_typeql(_params)`, `og_apply_role(max_rows)`는
**있는 줄 알고 쓰면 조용히 잘못 동작한다.**

---

## 7. 감사 로그의 `error_code` 컬럼

이름과 달리 **코드가 아니라 메시지 앞 200자**가 들어간다
([cypher/mod.rs:122](../../engine/src/cypher/mod.rs#L122),
[typeql/mod.rs:115](../../engine/src/typeql/mod.rs#L115)):

```rust
err.map(|e| e.chars().take(200).collect::<String>()).into()
```

```sql
SELECT at, lang, rows_out, duration_ms, error_code
  FROM og_data.og_audit
 WHERE error_code IS NOT NULL
 ORDER BY at DESC LIMIT 20;
```

`error_code`가 `NULL`이면 성공이다. 이것이 현재 유일하게 신뢰할 수 있는
성공/실패 구분자다.

> ⚠️ 감사 기록은 `og_cypher` / `og_typeql` 호출에서 **파싱 실패 시에만** 오류를
> 기록한다. 컴파일 실패나 실행 실패는 `error!`로 즉시 중단되어
> `audit(..., None)` 이 호출되지 않는다([cypher/mod.rs:140](../../engine/src/cypher/mod.rs#L140),
> [:149](../../engine/src/cypher/mod.rs#L149)) → [12_improvements_api.md](12_improvements_api.md) **API-31**.

---

## 8. 클라이언트를 위한 실용 지침

### 8.1 오류를 분류하는 방법 (현재 가능한 것)

```sql
-- 1) 실행 전에 파싱 검증 (오류를 던지지 않음)
SELECT og_cypher_check('MATCH (p:Person) RETURN p');
-- {"ok": true, "clauses": 2, "write": false}

-- 2) 컴파일 검증 + 구조화된 코드 (오류를 던지지 않음)
SELECT og_explain_error('default', 'MATCH (p:Person) RETURN p');
-- {"ok": true}

-- 3) 라벨 존재 여부는 별도로 확인해야 한다 (2)가 잡지 않는다
SELECT name FROM og_type_view WHERE graph = 'default';
```

### 8.2 재시도 판단

| 신호 | 재시도 가능? |
|---|---|
| `og_cypher_check().ok == false` | ❌ 질의를 고쳐야 함 |
| `og_explain_error().code == "CYPHER_PARSE_ERROR"` / `"UNKNOWN_FUNCTION"` / `"UNBOUND_VARIABLE"` | ❌ 질의를 고쳐야 함 |
| `og_explain_error().code == "INTERNAL"` | ⚠️ 엔진 문제 — 보고 대상 |
| `error_code`에 `no compiled graph in this backend` | ✅ `og_csr_build()` 후 재시도 |
| `error_code`에 `embedding request to '…' failed` | ✅ 외부 서비스 — 재시도 가능 |
| 그 외 `XX000` | ⚠️ 구별 불가 |

---

## 9. 금지 / 필수

- **금지**: SQLSTATE로 이 확장의 오류를 분류하려 하지 말 것 — 거의 전부 `XX000`이다.
- **금지**: `og_explain_error`의 `{"ok": true}`를 "질의가 옳다"로 읽지 말 것.
  라벨 오타는 잡히지 않는다(§4.2).
- **금지**: `UNKNOWN_LABEL` / `UNSUPPORTED_SYNTAX` 코드를 처리하는 분기를 작성하지 말 것 —
  현재 코드에서는 도달하지 않는다.
- **금지**: Studio의 HTTP `400`을 "클라이언트 잘못"으로 해석하지 말 것.
- **금지**: `og_data.og_audit.error_code`를 코드로 파싱하지 말 것 — 메시지 앞 200자다.
- **필수**: 실행 전 검증은 `og_cypher_check()` + `og_explain_error()` + 라벨 존재
  확인의 **세 단계**로 할 것.
- **필수**: §6의 "조용한 실패" 표를 클라이언트 구현 전에 읽을 것.
- **필수**: `cypher execution failed: …` 오류가 **컴파일된 SQL 전문**을 담는다는
  점을 감안해, 사용자에게 그대로 노출하지 말 것.

---

## 10. 관련 문서

- Cypher 문법 경계와 거부 메시지 → [03_cypher.md §4](03_cypher.md)
- TypeQL 거부 메시지 → [04_typeql.md §5](04_typeql.md)
- 에이전트 진단 함수 → [07_agent_interface.md §3](07_agent_interface.md)
- Bolt 코드 매핑 → [09_neo4j_compat.md §5.7](09_neo4j_compat.md)
- Studio 오류 응답 → [10_studio_http_api.md §2.3](10_studio_http_api.md)
- 개선 제안 → [12_improvements_api.md](12_improvements_api.md)

<!-- affects: api, backend, llm -->
<!-- requires-update: 02_api/12_improvements_api.md -->
