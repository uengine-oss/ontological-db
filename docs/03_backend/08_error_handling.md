# 오류 처리 전략 — `panic!` / `unwrap` / `expect` vs `ereport`

> **이 문서가 답하는 질문**
> - Rust의 오류가 PostgreSQL 사용자에게 어떤 형태로 도달하는가?
> - `error!` 매크로와 `unwrap()`은 무엇이 다른가?
> - `Result<T, String>` 하나로 버티는 이유와 그 대가는?
> - 오류가 Bolt / Studio / 에이전트까지 가는 경로는?
> - 오류에 "교정 정보"를 싣는다는 게 무슨 뜻인가?

---

## 1. 사실 — 세 가지 실패 방식

pgrx 확장에서 Rust 코드가 실패하는 방법은 셋뿐이고, 셋 다 결국 PostgreSQL의
`ereport(ERROR)`가 되어 트랜잭션을 abort시킨다.

| 방식 | 코드 | 사용자에게 보이는 것 | 트랜잭션 |
|---|---|---|---|
| `error!("…")` | pgrx 매크로 | `ERROR: <메시지>` — **우리가 쓴 문장** | abort |
| `unwrap()` / `expect("…")` | 표준 라이브러리 | `ERROR: called Option::unwrap() on a None value` 또는 `ERROR: <expect 문자열>` | abort |
| `Result<_, String>` 전파 | 함수 반환 | 최종적으로 `error!`로 변환됨 | abort |

`engine/Cargo.toml:39-45`가 `panic = "unwind"`를 강제한다 (dev/release 양쪽).
pgrx가 언와인드를 잡아 `ereport`로 바꾸므로, 패닉이 백엔드를 죽이지는 않는다.
하지만 **메시지가 Rust 내부 문구**가 된다.

---

## 2. 결정 — `error!`가 기본이다

`error!` 사용처 (파일별 건수):

| 파일 | `error!` |
|---|---|
| `cypher/mod.rs` | 19 |
| `catalog/types.rs` | 18 |
| `interop/mod.rs` | 12 |
| `compat/ddl.rs` | 12 |
| `storage/mod.rs` | 11 |
| `vector/mod.rs` | 10 |
| `compat/genai.rs` | 8 |
| `typeql/mod.rs` | 7 |
| `storage/traverse.rs` | 6 |
| `id.rs`, `catalog/labeling.rs`, `agent/mod.rs` | 각 3 |
| `adapters/rdf.rs` | 2 |
| `cypher/views.rs` | 1 |

특징: **`error!`는 사용자 입력으로 도달 가능한 지점에 집중돼 있다.**
`typeql/*`(파서·컴파일러·쓰기)에는 `error!`가 0건이고, 전부 `Result<_, String>`으로
`typeql/mod.rs`까지 전파된 뒤 거기서 한 번만 `error!`가 된다 (`typeql/mod.rs:56-62`).

### 2.1 좋은 오류의 예

메시지가 **무엇이 틀렸는지 + 무엇을 하면 되는지**를 함께 담는다.

```rust
// catalog/types.rs:129-138
error!("type '{name}' does not exist. did you mean: {}", hint.join(", "));

// cypher/mod.rs:286-290
error!("cannot delete node {id}: it still has {deg} relationship(s). use DETACH DELETE");

// catalog/types.rs:532-536
error!("cannot add required property '{prop}' to '{type_name}': {n} existing instance(s) \
        would violate it. add it as optional, backfill, then tighten.");

// cypher/mod.rs:627-632
error!("cannot add label '{l}' to this node: a node's type is part of its identifier here, \
        so it cannot gain one after creation. To rename a class, write `REMOVE n:Old SET n:New` …");

// compat/genai.rs:102-106
error!("genai.vector.encode is disabled. It makes an outbound HTTP request from the database, \
        so it is off until that is chosen deliberately: SELECT og_set_setting('genai.enabled', 'on')");

// cypher/compile.rs:1558-1565
"unknown function '{other}'. supported: count, sum, avg, min, max, collect, id, elementId, …"
```

이건 취향이 아니라 **스펙 요구사항**이다 (spec 003 FR-008, spec 008 FR-008).
`agent/mod.rs:1-8`이 이유를 명시한다:

> The premise: the entity writing Cypher against this database is increasingly a
> language model, and it fails differently from a human. … So the database owes it
> three things: an accurate machine-readable schema, errors that carry their own
> correction, and limits that stop a bad query before it stops the server.

### 2.2 편집 거리 힌트

`nearest_type_names`(`catalog/types.rs:236-261`)가 레벤슈타인 거리로 후보를 고른다.
컷오프는 `max(target.len()/2, 2)`, 최대 3개.

`fuzzystrmatch` 확장에 의존하지 않으려고 Rust로 직접 구현했다 (`types.rs:233-235`,
`edit_distance` `types.rs:264-284`).

호출처: `types::type_id`(`types.rs:130-137`), `types::resolve_label_set`(`types.rs:166-173`),
`typeql/compile.rs:120-129`, `typeql/write.rs:180-187`, `typeql/schema.rs:178-185`,
`compat/procs.rs:275-290`(`missing_index`).

### 2.3 오류가 아닌 것 — `notice!`

없는 라벨은 **오류가 아니다.** `catalog/types.rs:161-176`:

```rust
// A label nothing has ever been written under is not an error — in Cypher it
// simply matches nothing, and a caller that probes for a label before creating it
// relies on that. The spelling hint still goes out, as a notice.
pgrx::notice!("label '{name}' does not exist in graph '{graph}' — matching nothing. \
               did you mean: {}", near.join(", "));
return Ok(LabelMatch::Nothing);
```

같은 원칙으로 `types::ensure_alias_view`(`types.rs:89-98`)는 뷰 생성 실패를
`pgrx::log!`로만 남긴다 — 별칭 뷰는 편의 기능이고, 이름 충돌 때문에 타입이 존재하지 못하면 안 된다.

---

## 3. 사실 — `unwrap` / `expect` / `panic!` 현황

전수 조사:

```
$ grep -rn "unwrap()\|expect(\|panic!" engine/src --include=*.rs | wc -l
202
```

파일별 상위:

| 파일 | 건수 |
|---|---|
| `catalog/types.rs` | 38 |
| `storage/mod.rs` | 25 |
| `storage/stats.rs` | 20 |
| `typeql/parser.rs` | 18 |
| `agent/mod.rs` | 14 |
| `catalog/labeling.rs` | 13 |
| `vector/mod.rs` | 10 |
| `cypher/parser.rs` | 10 |
| `interop/mod.rs` | 9 |
| `cypher/views.rs` | 7 |
| `adapters/mod.rs` | 7 |
| `typeql/schema.rs` / `adapters/rdf.rs` | 각 6 |
| `storage/adjacency.rs` | 5 |
| 나머지 | 각 0–4 |

### 3.1 세 가지 부류로 나뉜다

**(a) 테스트 코드 안 — 무해**

`cypher/parser.rs:1133-1177`, `cypher/lexer.rs:266-302`, `typeql/parser.rs:964+`,
`bolt/src/packstream.rs:285+` 등. `#[cfg(test)]` 안의 `unwrap()`/`panic!()`은
테스트 실패 표현 수단이다.

**(b) 내부 불변식 — 방어적, 사용자 입력으로 도달 불가**

```rust
// storage/traverse.rs:272
let pos = |id: i64| ids.binary_search(&id).expect("id present by construction") as u32;
```

`ids`는 방금 같은 루프에서 만든 것이므로 실패할 수 없다. `expect` 문자열이 그 사실을 말한다.

```rust
// bolt/src/session.rs:93
.map(|c| u32::from_be_bytes(c.try_into().unwrap()))
```

`chunks(4)`의 결과이므로 길이가 보장된다.

**(c) SPI 결과 언랩 — 사용자 입력으로 도달 가능**

이게 문제다. 전형적인 형태:

```rust
// storage/mod.rs:25-33  — alloc_id
let local = crate::spiu::one_mut::<i64>("INSERT INTO og_data.og_id_alloc …")
    .expect("id allocation failed")
    .unwrap();                                   // ← Option 언랩
```

```rust
// cypher/views.rs:44
.expect("property lookup failed");

// storage/mod.rs:168
.expect("property lookup failed");

// catalog/labeling.rs:44
let id: i32 = row.get(1).unwrap().unwrap();     // 이중 언랩

// catalog/types.rs:117-118
.expect("graph lookup failed")
.unwrap_or_else(|| error!("graph '{name}' does not exist"))   // ← 이건 좋은 패턴
```

`catalog/types.rs:112-119`의 마지막 형태가 **권장 패턴**이다:
SPI 실패는 `expect`(진짜 내부 오류), 결과 없음은 `error!`(사용자에게 설명).

문제가 되는 건 그 구분이 없는 곳이다. 예를 들어 `catalog/labeling.rs:44`의
`row.get(1).unwrap().unwrap()`은 SPI 오류든 NULL이든 똑같이
`called Option::unwrap() on a None value`를 낸다.

자세한 목록과 심각도는 [`11_improvements_code.md`](11_improvements_code.md) `CODE-25`.

### 3.2 `panic!` 직접 사용

프로덕션 코드 경로에는 `panic!()` 직접 호출이 없다. `#[cfg(test)]` 안의
`else { panic!() }` 패턴만 있다 (예: `cypher/parser.rs:1146,1147`).

### 3.3 `error!` 안의 언랩 — 좋은 관용구

`id.rs:31-45`가 명시적으로 이 선택을 문서화한다:

```rust
/// Compose an identifier. Panics (→ `ereport(ERROR)` via pgrx) on overflow so a
/// silently truncated id can never reach storage.
pub fn make_id(shard: i32, type_id: i32, local: i64) -> i64 {
    if !(0..=MAX_SHARD_ID).contains(&shard) { error!("shard id {shard} out of range …"); }
    ...
}
```

조용히 잘린 식별자보다 시끄러운 실패가 낫다.

---

## 4. 사실 — `Result<T, String>` 계층

7개의 타입 별칭이 전부 `Result<T, String>`이다:

| 별칭 | 위치 |
|---|---|
| `CResult<T>` | `cypher/compile.rs:149` |
| `PResult<T>` | `cypher/parser.rs:20` |
| `CResult<T>` (pub) | `typeql/compile.rs:21` |
| `QResult<T>` | `typeql/mod.rs:27` |
| `PResult<T>` | `typeql/parser.rs:17` |
| `SResult<T>` (pub) | `typeql/schema.rs:25` |
| `WResult<T>` | `typeql/write.rs:18` |

**오류 타입이 없다. 오류 코드도 없다. 원인 사슬도 없다.**

### 4.1 대가 1 — 분류 불가

Bolt 게이트웨이가 Neo4j 오류 코드를 복원해야 하는데,
분류 정보가 없으므로 **영어 메시지 부분 문자열을 본다** (`bolt/src/session.rs:578-593`):

```rust
let code = if message.contains("not supported") || message.contains("expected")
              || message.contains("unknown label") || message.contains("is not defined") {
    "Neo.ClientError.Statement.SyntaxError"
} else if message.contains("does not exist") {
    "Neo.ClientError.Database.DatabaseNotFound"
} else if message.contains("permission denied") {
    "Neo.ClientError.Security.Forbidden"
} else {
    "Neo.ClientError.Statement.ArgumentError"
};
```

**엔진의 오류 문구를 바꾸면 Bolt 오류 코드가 조용히 바뀐다.**
`"unknown label"`은 실제로 코드에 없는 문구다 — 현재 코드는
`"label '…' does not exist"`(notice)와 `"type '…' does not exist"`를 쓴다.
즉 첫 분기의 세 번째 조건은 이미 죽어 있다. → `CODE-11`.

### 4.2 대가 2 — SQLSTATE가 전부 같다

`error!`는 pgrx 기본 SQLSTATE(`XX000` internal_error)로 나간다.
`ERRCODE_INVALID_PARAMETER_VALUE`, `ERRCODE_UNDEFINED_OBJECT` 같은 표준 코드를
쓰는 곳이 없으므로, SQL 클라이언트가 `SQLSTATE`로 분기할 수 없다.

### 4.3 대가 3 — 컨텍스트 유실

`String`은 원인을 감싸지 못하므로 `format!("{context}: {e}")`로 손수 붙인다:

```rust
// typeql/write.rs:155
.map_err(|e| format!("put lookup failed: {e}\n--- SQL ---\n{sql}"))?
```

잘 하고 있는 곳도 있지만 규약이 아니라 관례다.

### 4.4 얻은 것

`String`은 **번역·조합·프롬프트 삽입이 자유롭다.**
`compile.rs:1558-1566`처럼 지원 목록 전체를 메시지에 넣는 것이 자연스럽다.
`Compiler.notes`(`compile.rs:120-122`)도 같은 철학이다 — 진단 문자열을 그냥 모은다.

---

## 5. 사실 — 컴파일 SQL을 오류에 붙인다

실행 실패 시 **생성된 SQL 전문**을 메시지에 넣는다:

```rust
// cypher/mod.rs:145-152
.unwrap_or_else(|e| error!("cypher execution failed: {e}\n--- compiled SQL ---\n{sql}"));

// typeql/mod.rs:434-441
.unwrap_or_else(|e| error!("typeql execution failed: {e}\n--- compiled SQL ---\n{sql}"));
```

컴파일러 버그를 즉시 재현 가능하게 만드는 장치다.

> **보안 고려**: 컴파일된 SQL에는 사용자 값이 들어 있지 않다(파라미터는 `$1`).
> 하지만 **스키마 정보(테이블 이름, 컬럼 이름, 타입 id)**는 노출된다.
> 오류 메시지는 `og_data.og_audit`에도 200자까지 기록된다 (`cypher/mod.rs:131`).

---

## 6. 사실 — 오류가 사용자에게 도달하는 경로

```
Rust error!/panic
   ↓ pgrx
PostgreSQL ereport(ERROR)  ← 트랜잭션 abort
   ↓
   ├─ psql            → "ERROR: <메시지>"
   ├─ Bolt 게이트웨이  → Failure::from_pg() → FAILURE {code, message}
   │                     bolt/src/session.rs:578-601
   ├─ Studio          → portal/server의 pg 드라이버 오류
   └─ 에이전트 표면    → og_explain_error() (spec 008)
```

**감사 로그**: `og_cypher()`와 `og_typeql()`은 성공/실패 양쪽에서
`og_data.og_audit`에 기록한다 (`cypher/mod.rs:122-135`, `typeql/mod.rs:115-128`):

```sql
INSERT INTO og_data.og_audit (query, lang, rows_out, duration_ms, error_code)
VALUES ($1, 'cypher', $2, $3, $4)
```

`error_code` 컬럼에는 코드가 아니라 **오류 메시지 앞 200자**가 들어간다
(`cypher/mod.rs:131`: `err.map(|e| e.chars().take(200).collect::<String>())`).
컬럼 이름과 내용이 불일치한다. → `CODE-26`.

이 INSERT는 `.ok()`로 실패를 삼킨다 — 감사 기록 실패가 질의를 실패시키지 않는다.
읽기 전용 트랜잭션(`og_apply_role`로 `default_transaction_read_only = on`)에서는
**모든 감사 기록이 조용히 사라진다.**

---

## 7. 사실 — 진단 표면

오류를 낸 뒤 사용자가 물어볼 수 있는 것들:

| 함수 | 하는 일 | 위치 |
|---|---|---|
| `og_cypher_check(query)` | 파싱만. `{ok, clauses, write}` 또는 `{ok:false, error}` | `cypher/mod.rs:699-709` |
| `og_cypher_sql(graph, q)` | 컴파일된 SQL | `cypher/mod.rs:74-80` |
| `og_cypher_explain(graph, q, analyze)` | `{columns, sql, plan}` | `cypher/mod.rs:676-696` |
| `og_diagnose_empty(...)` | 패턴을 한 요소씩 늘려가며 행 수를 세고, **처음 0이 되는 지점**을 지목 | `cypher/mod.rs:747-803` |
| `og_explain_error(...)` | 오류 교정 (spec 008) | `agent/mod.rs` |
| `og_typeql_check(query)` | 파싱만 | `typeql/mod.rs:69-78` |

`diagnose_pattern`(`cypher/mod.rs:747-803`)이 특히 좋은 예다.
`MATCH`가 빈 결과를 낸 이유를 찾기 위해 패턴을 점진적으로 컴파일해 실행하고,
행이 0이 되는 첫 지점에서 멈춰 이렇게 답한다:

```json
{"verdict": "this is where the match became empty",
 "hint": "check the label spelling, the relationship type, and the direction of the arrow at this step"}
```

패턴은 다 맞았는데 `WHERE`가 다 걸러낸 경우도 구분한다 (`cypher/mod.rs:796-801`).

---

## 8. 결정 요약

| 결정 | 근거 | 대가 |
|---|---|---|
| `error!`가 기본, 메시지에 교정 정보 포함 | spec 003 FR-008, spec 008 | 없음 — 이 코드베이스의 강점 |
| 없는 라벨은 오류가 아니라 빈 결과 + notice | `catalog/types.rs:161-165` | 오타를 즉시 못 잡음 (notice는 놓치기 쉬움) |
| 오류 타입 없이 `Result<T, String>` | 관례 | Bolt 코드 매핑이 문자열 매칭 (`CODE-11`) |
| 컴파일 SQL을 오류에 첨부 | 디버깅 | 스키마 정보 노출 |
| 감사 기록 실패는 삼킨다 | 감사가 질의를 막으면 안 됨 | 읽기 전용 세션에서 감사 전무 |
| pgrx `panic = "unwind"` | `Cargo.toml:39-45` | 언랩 실패가 Rust 내부 문구로 노출 |

---

## 금지 / 필수

- **금지**: 사용자 입력으로 도달 가능한 경로에서 결과 없음(`None`)을 `unwrap()`으로 처리하는 것.
  `unwrap_or_else(|| error!("<설명>"))`을 쓴다 (`catalog/types.rs:112-119` 참고).
- **금지**: `expect()` 문자열을 사용자 대상 문장으로 쓰는 것. `expect`는
  **내부 불변식 위반**에만 쓰고, 문자열은 그 불변식을 설명한다
  (`storage/traverse.rs:272` 참고).
- **금지**: 오류 메시지 문구를 바꾸면서 `bolt/src/session.rs:578-593`의 매칭 목록과
  `engine/tests/sql/*.sql`의 `EXPECT_ERROR` 주석을 확인하지 않는 것.
- **금지**: 성공적으로 실행된 부작용을 오류 하나 때문에 되돌리지 않는 것.
  트랜잭션이 abort하므로 자동으로 되돌아가지만, 백엔드-로컬 상태
  (`stats`, `PLAN_CACHE`, `CSR`)는 되돌아가지 않는다.
- **필수**: 새 오류 메시지는 **무엇이 틀렸는지 + 무엇을 하면 되는지**를 함께 적는다.
  타입/프로퍼티/인덱스 이름이 안 맞으면 `nearest_type_names()` 힌트를 붙인다.
- **필수**: 새 `#[pg_extern]` 함수의 실패 경로가 `error!`로 끝나는지 확인한다.
  `Result`를 반환하면 pgrx가 그것을 어떻게 처리하는지 명시적으로 정한다.

<!-- affects: backend, api, operations -->
<!-- requires-update: 02_api/, 08_operations/ -->
