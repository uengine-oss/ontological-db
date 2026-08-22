# 주입(Injection) 표면 — 질의 컴파일러 감사

> **이 문서가 답하는 질문**
> - "사용자 값은 절대 SQL 텍스트로 보간하지 않는다"(spec 003 FR-026)는 주장이
>   코드에서 실제로 성립하는가?
> - 식별자(테이블/컬럼명)는 어떻게 생성되고 인용되는가?
> - **방어가 없는 지점은 어디인가?**
> - 2차(second-order) 주입은 가능한가?

이 문서는 감사 결과를 **확인된 방어**와 **확인된 결함**으로 나눈다.
모든 항목에 `파일:줄` 근거가 붙어 있다.

---

## 0. 요약표

| 분류 | 건수 | 대표 사례 |
|---|---|---|
| 확인된 방어 (값 바인딩) | 4 | jsonb 파라미터 `$1` 단일 바인딩 |
| 확인된 방어 (식별자 화이트리스트/인용) | 6 | `column_name`, `map_data_type`, `check_dir`, `sanitize`, `quote_ident`, `sql_str` |
| **1차 주입 (원시 SQL 인자)** | **3** | `og_vector_search.filter`, `og_enable_rls.policy_expr`, `og_map_table` |
| **2차 주입 (카탈로그 경유)** | **4** | `storage_table`, `data_type`, `column_name`, `embedding.prop` |
| 조건부 위험 (GUC 의존) | 1 | `standard_conforming_strings` |

---

## 1. 확인된 방어 — 값 바인딩

### D-01. 사용자 값은 jsonb 파라미터 `$1` 하나로만 들어간다

```rust
// engine/src/cypher/compile.rs:16-18
/// The bound jsonb parameter holding user `$params`.
pub const PARAM: &str = "$1";
```

파라미터 참조는 이렇게 컴파일된다:

```rust
// engine/src/cypher/compile.rs:1155-1161
Expr::Param(p) => {
    let base = format!("({PARAM} ->> {})", sql_str(p));
    match hint {
        Some(t) if t != "text" => format!("{base}::{t}"),
        _ => base,
    }
}
```

`p`(파라미터 **이름**)는 `sql_str`로 이스케이프되고, **값은 SQL 텍스트에
전혀 나타나지 않는다.** 실행 시 바인딩:

```rust
// engine/src/cypher/mod.rs:145-152
fn exec_json(sql: &str, params: &Value) -> Vec<Value> {
    Spi::connect(|client| {
        let table = client
            .select(sql, None, &[JsonB(params.clone()).into()])
```

`&[JsonB(params.clone()).into()]` — 파라미터 배열 원소가 정확히 하나다.
**spec 003 FR-026의 주장은 이 두 줄로 성립한다.**

### D-02. 쓰기 경로도 동일하다

```rust
// engine/src/storage/mod.rs:42-46
/// Build the column list / value expressions for a property payload.
///
/// Declared properties become real columns; everything else is funnelled into
/// `__ext`.  All values are extracted from ONE bound jsonb parameter, so no
/// user value is ever interpolated into SQL text (spec 003 FR-026).
```

실제 구현:

```rust
// engine/src/storage/mod.rs:208-220
let lit = quote_json_key(&name);
let expr = if dtype.ends_with("[]") {
    let elem = dtype.trim_end_matches("[]");
    format!(
        "(SELECT array_agg(x)::{dtype} FROM jsonb_array_elements_text({param}->{lit}) AS t(x_raw), \
         LATERAL (SELECT t.x_raw::{elem}) AS c(x))"
    )
} else {
    format!("({param}->>{lit})::{dtype}")
};
```

그리고 실행:

```rust
// engine/src/storage/mod.rs:285-286
Spi::run_with_args(&sql, &[nid.into(), JsonB(props).into()])
```

노드 생성(`create_node_inner:271-286`), 프로퍼티 갱신
(`set_node_props_inner:298-326`), 엣지 생성(`create_edge_inner:429-442`),
엣지 프로퍼티(`set_edge_props_inner:329-347`) 모두 같은 형태다.
**노드/엣지 값이 SQL 텍스트에 들어가는 경로는 발견되지 않았다.**

### D-03. Bolt 파라미터도 그대로 위임된다

```rust
// bolt/src/session.rs:544-546
/// Parameters travel to `og_cypher()` as jsonb, never interpolated into the
/// query text — the injection guarantee is spec 003's FR-026, unchanged here.
```

```rust
// bolt/src/session.rs:294-297
pg.query(
    "SELECT og_cypher($1::text, $2::text, $3::text::jsonb)::text",
    &[&graph, &query, &params_json],
)
```

게이트웨이는 Cypher를 파싱하지 않고, 질의문과 파라미터를 각각 바인딩한다.
**확인된 방어.**

### D-04. Studio도 파라미터화되어 있다 (`/api/sql` 제외)

```js
// portal/server/index.js:190-194
const r = await client.query('SELECT og_cypher($1,$2,$3) AS row', [
  graph,
  query,
  JSON.stringify(params),
]);
```

`/api/cypher`, `/api/explain`, `/api/diagnose`, `/api/expand`, `/api/schema`는
전부 파라미터 바인딩을 쓴다. **예외는 `POST /api/sql`(`index.js:299`)뿐이며
그것은 설계상 원시 SQL 실행기다** ([`06_network_exposure.md`](06_network_exposure.md) SEC-01).

---

## 2. 확인된 방어 — 식별자 생성과 인용

### D-05. 물리 컬럼명은 화이트리스트 변환된다

```rust
// engine/src/catalog/types.rs:46-66
/// Physical column name for a declared property. Deterministic and injection-safe.
pub fn column_name(prop: &str) -> String {
    let mut s = String::with_capacity(prop.len() + 2);
    s.push_str("p_");
    for c in prop.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c.to_ascii_lowercase());
        } else if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else {
            s.push('_');
        }
    }
    s
}
```

출력에는 `'`, `"`, `;`, 공백, 괄호가 절대 포함되지 않는다.
`p_` 접두사로 시스템 컬럼(`id`, `src`, `dst`, `__ext`)과 충돌하지도 않는다.
**컬럼명 주입은 불가능하다** — 다만 §5의 충돌 문제는 별개다.

### D-06. 테이블명은 정수에서만 만들어진다

```rust
// engine/src/catalog/types.rs:68-74
pub fn node_table(type_id: i32) -> String { format!("og_data.n_{type_id}") }
pub fn edge_table(type_id: i32) -> String { format!("og_data.e_{type_id}") }
```
```rust
// engine/src/cypher/views.rs:23-29
pub fn node_view(tid: i32) -> String { format!("og_data.v_{tid}") }
pub fn edge_view(tid: i32) -> String { format!("og_data.ve_{tid}") }
```
```rust
// engine/src/typeql/schema.rs:49
    format!("og_data.a_{tid}")
```

`i32`만 보간된다. **테이블명 주입 불가.**

### D-07. 데이터 타입은 폐쇄 화이트리스트다

```rust
// engine/src/catalog/types.rs:13-44
pub fn map_data_type(decl: &str) -> String {
    let d = decl.trim().to_ascii_lowercase();
    match d.as_str() {
        "string" | "text" | "str" => "text".into(),
        // … 고정 목록 …
        other => {
            if let Some(rest) = other.strip_prefix("vector(") {
                if let Some(dims) = rest.strip_suffix(')') {
                    if dims.chars().all(|c| c.is_ascii_digit()) && !dims.is_empty() {
                        return format!("vector({dims})");
                    }
                }
            }
            error!("unsupported property type '{decl}'. …")
        }
    }
}
```

`vector(N)`의 `N`은 `is_ascii_digit()` 전수 검사를 통과해야 한다.
**타입명 주입 불가.**

### D-08. 방향(`dir`) 리터럴은 3원소 집합으로 검증된다

```rust
// engine/src/storage/traverse.rs:35-64
fn check_dir(dir: &str) -> &str {
    match dir {
        "o" | "i" | "b" => dir,
        _ => error!("direction must be 'o', 'i' or 'b', not '{dir}'"),
    }
}
…
fn adj_where(dir: &str, alias: &str, types_param: &str) -> String {
    let d = match check_dir(dir) {
        "b" => format!("{alias}.dir IN ('o','i')"),
        other => format!("{alias}.dir = '{other}'::\"char\""),
    };
    format!("{d} AND ({types_param}::int4[] IS NULL OR {alias}.etype = ANY({types_param}::int4[]))")
}
```

주석(`traverse.rs:52-57`)이 이 판단을 명시적으로 문서화하고 있다.
관계 타입 목록은 바운드 파라미터로 남는다. **확인된 방어.**

### D-09. 인덱스명은 영숫자로 정규화된다

```rust
// engine/src/compat/ddl.rs:280-282
fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}
```
사용처: `compat/ddl.rs:160`(`uq_{sub}_{sanitize(name)}`), `:263`(`ftx_{sub}_{…}`).
**Neo4j 호환 DDL의 인덱스명 주입 불가.**

### D-10. 문자열·식별자 인용 유틸

```rust
// engine/src/cypher/compile.rs:1581-1591
/// SQL identifier with proper quoting. …
pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// SQL string literal with proper escaping.
pub fn sql_str(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
```
동일 구현: `engine/src/typeql/compile.rs:616-618` (`lit_str`),
`engine/src/typeql/write.rs:686-688` (`quote_ident`),
`engine/src/storage/mod.rs:224-226` (`quote_json_key`),
`engine/src/catalog/types.rs:104-106` (`alias_view_name`).

라벨·프로퍼티 이름·`RETURN` 별칭이 SQL에 나타날 때 이 함수들을 거친다:
`compile.rs:988`(`__ext->>`), `:1013`, `:1101`, `:1119`, `:607`, `:632`.
**이스케이프 자체는 올바르다** — 단 §6의 GUC 조건이 붙는다.

### D-11. `og_apply_role`의 GUC 값은 숫자·불리언만 추출된다

`engine/src/agent/mod.rs:426, 429, 432, 437` — `as_i64()` / `as_bool()`로만
꺼내므로 문자열이 `SET` 문에 보간되지 않는다. **GUC 주입 불가.**

### D-12. `og_load_rdf`는 파일도 URL도 열지 않는다

```rust
// engine/src/adapters/mod.rs:39-44
#[pg_extern]
fn og_load_rdf(graph: &str, document: &str, format: default!(&str, "'turtle'")) -> JsonB {
    let report = rdf::load(graph, document, format);
```

`document`는 **RDF 본문 텍스트**다. `engine/src/adapters/rdf.rs`(883줄)에는
`std::fs`, `File`, `ureq`, `reqwest` 참조가 없다(전수 검색 확인). 파서는
Turtle / N-Triples만 지원하며 **XML 파서가 없으므로 XXE 표면도 없다.**
→ 경로 순회·SSRF·XXE **모두 해당 없음. 확인된 방어.**

---

## 3. 확인된 결함 — 1차 주입 (원시 SQL을 받는 인자)

세 함수가 SQL 조각을 **설계상** 받는다. 문제는 이것이 `#[pg_extern]`으로
`PUBLIC`에 노출되어 있고, PostgREST RPC·Bolt·Studio를 통해 도달 가능하며,
"신뢰된 입력만"이라는 제약이 코드에도 `docs/api.md`에도 적혀 있지 않다는 점이다.

### F-01. `og_vector_search(..., filter)` — CWE-89

```rust
// engine/src/vector/mod.rs:88-90
/// `filter` is a SQL boolean fragment evaluated on the same relation as the
/// index, which is what keeps it a push-down rather than a post-filter.
```
```rust
// engine/src/vector/mod.rs:115-118
let where_sql = match filter {
    Some(f) if !f.trim().is_empty() => format!("AND ({f})"),
    _ => String::new(),
};
```
```rust
// engine/src/vector/mod.rs:126-132
let sql = format!(
    "SELECT v.id, {score}::float8 AS score, {json_fn}(v.id) AS entity
       FROM {view} v
      WHERE v.{col} IS NOT NULL {where_sql}
      ORDER BY v.{col} {op} $1::vector
      LIMIT {k}"
);
```

**재현 조건**: `og_vector_search` 실행 권한을 가진 호출자가 `filter` 인자를
제어할 수 있으면 된다. `AND (…)` 안에 스칼라 서브쿼리를 넣어 다른 테이블의
값을 불리언으로 관측하는 형태(블라인드 추출)가 성립한다. 인자가 있는
SPI 호출은 준비된 문(prepared statement)을 쓰므로 **다중 문장 실행은 되지
않는 것으로 보이나, 이는 pgrx 내부 동작에 의존하므로 미확인이다.**

**참고**: 같은 함수가 Neo4j 호환 프로시저 경로에서 호출될 때는 `filter`를
넘기지 않는다(`engine/src/compat/procs.rs:189-196` — 5인자 호출). 그 경로는 안전하다.

### F-02. `og_enable_rls(..., policy_expr)` — CWE-89

`engine/src/interop/mod.rs:27-29`. §[`03_rls_and_isolation.md`](03_rls_and_isolation.md) §1 참조.
`Spi::run`은 인자 없는 SPI 호출이므로 **다중 문장이 실행될 가능성이 F-01보다 높다(미확인).**

### F-03. `og_map_table(graph, source_table, type_name, id_column, property_map)` — CWE-89

```rust
// engine/src/interop/mod.rs:74-101
let mut cols: Vec<String> = vec![format!(
    "(og_make_id(0, {tid}, ({id_column})::int8)) AS id"
)];
for (prop, col) in &map {
    let Some(src_col) = col.as_str() else { … };
    …
    cols.push(format!("({src_col})::{dtype} AS {}", types::column_name(prop)));
}
…
Spi::run(&format!("DROP TABLE IF EXISTS {table} CASCADE")).ok();
Spi::run(&format!(
    "CREATE VIEW {table} AS SELECT {} FROM {source_table}",
    cols.join(", ")
))
```

`source_table`, `id_column`, 그리고 `property_map`의 **값**(`src_col`)이
모두 인용 없이 보간된다. 추가로 `interop/mod.rs:97`의
`DROP TABLE IF EXISTS {table} CASCADE`는 **기존 네이티브 타입 테이블을
확인 없이 파괴한다** — 데이터 손실 경로다.

---

## 4. 확인된 결함 — 2차 주입 (카탈로그 경유)

`og_catalog`의 `text` 컬럼 값이 동적 SQL로 보간된다. 이 컬럼들에는 `CHECK`
제약이 없다(`engine/sql/bootstrap.sql:35-46, 87-100`).

### F-04. `og_node_json` / `og_edge_json` — `%I`가 아니라 `%s` — CWE-89

```sql
-- engine/sql/access.sql:220
    EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t.storage_table)
-- engine/sql/access.sql:249
    EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t.storage_table)
```

`t.storage_table`은 `og_catalog.type.storage_table`(제약 없는 `text`)에서 온다.
`%I`를 쓸 수 없는 이유는 값이 `og_data.n_5`처럼 **스키마 한정 이름**이라
`%I`가 통째로 인용해버리기 때문이다. 올바른 형태는
`format('… FROM %I.%I x …', 'og_data', 'n_' || t.type_id)`이다.

**재현 조건**: `og_catalog.type`에 `UPDATE` 권한을 가진 주체가
`storage_table`을 조작한 뒤, **다른(더 높은 권한의) 역할이** `og_cypher`나
`og_node_json`을 호출하면 그 역할 권한으로 주입된 SQL이 실행된다.
`og_node_json`은 `SECURITY DEFINER`가 아니므로 자기 자신에 대한 권한 상승은
아니지만, 관리자를 유인하는 형태의 상승은 성립한다.

### F-05. `plan_props` — `data_type` 보간 — CWE-89

`engine/src/storage/mod.rs:209-217` (§1 D-02 인용 참조). `dtype`는
`og_catalog.property.data_type`에서 온다. 정상 경로에서는 `map_data_type`이
화이트리스트를 통과시켰지만, 카탈로그를 직접 수정하면 임의 텍스트가 된다.

### F-06. `copy_inherited_properties` / `ensure_view` — `column_name`·`data_type` 보간

```rust
// engine/src/catalog/types.rs:482-488
Spi::run(&format!("ALTER TABLE {child_table} ADD COLUMN IF NOT EXISTS {col} {dtype}"))
    .expect("inherited column add failed");
if required {
    Spi::run(&format!("ALTER TABLE {child_table} ALTER COLUMN {col} SET NOT NULL")).ok();
}
if is_key {
    Spi::run(&format!("CREATE UNIQUE INDEX ON {child_table} ({col})")).ok();
}
```
```rust
// engine/src/cypher/views.rs:110-116
for (col, dt) in props.values() {
    if own.contains(col) {
        cols.push(col.clone());
    } else {
        cols.push(format!("NULL::{dt} AS {col}"));
    }
}
```

### F-07. `og_stale_embeddings` — **이스케이프 없는 문자열 리터럴** — CWE-89

```rust
// engine/src/vector/mod.rs:335-341
&format!(
    "SELECT x.id FROM {table} x
      LEFT JOIN og_data.og_embedding_state s
        ON s.entity_id = x.id AND s.prop = '{prop}'
      WHERE x.{scol} IS NOT NULL
        AND (x.{ecol} IS NULL
             OR s.source_hash IS DISTINCT FROM md5(x.{scol}::text))"
),
```

`'{prop}'` — `sql_str`을 쓰지 않았다. `prop`은 `og_catalog.embedding.prop`이며,
그 값은 `og_add_embedding(graph, type_name, prop, dims, …)`의 `prop` 인자가
**가공 없이** 저장된 것이다(`engine/src/vector/mod.rs:66-73`).

**재현 조건**: 작은따옴표를 포함한 프로퍼티 이름으로 `og_add_embedding`을
호출한 뒤 `og_stale_embeddings(graph)`를 호출한다. `og_add_embedding`은 컬럼명은
`column_name()`으로 정규화하지만(D-05) **프로퍼티 이름 자체는 원문 저장**하므로
따옴표가 카탈로그에 들어간다. 이것이 이 문서군에서 확인된 **유일한
"완전히 이스케이프되지 않은" 문자열 보간**이다.

---

## 5. 확인된 결함 — 식별자 충돌 (주입은 아니나 격리 위반)

`column_name`(D-05)은 서로 다른 프로퍼티를 같은 컬럼으로 접을 수 있다.

| 프로퍼티 이름 | `column_name` 결과 |
|---|---|
| `a b` | `p_a_b` |
| `a-b` | `p_a_b` |
| `a.b` | `p_a_b` |
| `A_B` | `p_a_b` |

`og_catalog.property`의 유일 제약은 `UNIQUE (type_id, name)`이며
(`engine/sql/bootstrap.sql:99`) `column_name`에는 유일 제약이 없다. 그리고
DDL은 `ADD COLUMN IF NOT EXISTS`(`engine/src/catalog/types.rs:550`)이므로
**두 번째 프로퍼티는 조용히 첫 번째의 컬럼을 재사용한다.**

결과: `a-b`에 쓴 값이 `a b`를 덮어쓰고, `a b`를 읽으면 `a-b`의 값이 나온다.
소스 주석(`catalog/types.rs:46-52`)은 한글 등 유니코드 충돌을 피하려고
비-ASCII 문자를 보존한다고 설명하지만, **ASCII 구두점 충돌은 남아 있다.**

---

## 6. 조건부 위험 — `standard_conforming_strings`

`sql_str` / `lit_str` / `quote_json_key`는 `'` → `''` 치환만 한다.
이는 `standard_conforming_strings = on`(PostgreSQL 9.1+ 기본값)에서만 안전하다.
`off`이면 `\'`가 이스케이프로 해석되어 탈출이 가능하다.

**감사 결과**: 저장소 어디에도 `standard_conforming_strings`를 설정하거나
확인하는 코드가 없다(전수 검색). 또한 `E''` 접두사나
`quote_literal()` 위임도 쓰지 않는다.

**실질 위험도는 낮다** — 기본값이 `on`이고 명시적으로 끄는 것은 드물다.
다만 함수에 `SET search_path`도 `SET standard_conforming_strings`도 없어
세션 상태에 의존한다는 사실은 기록해 둔다.

---

## 7. 동적 SQL 전수 목록 (감사 인벤토리)

`Spi::run(&format!(...))` 및 동등 호출로 SQL 텍스트를 조립하는 지점.
"안전 근거" 열이 비어 있는 행이 §3·§4의 결함이다.

| 파일:줄 | 무엇을 만드는가 | 안전 근거 |
|---|---|---|
| `catalog/types.rs:92-93` | `DROP/CREATE VIEW og_data."<Type>"` | `alias_view_name` 인용(`:104-106`) — 단 §8 참조 |
| `catalog/types.rs:101` | `DROP VIEW IF EXISTS …` | 동상 |
| `catalog/types.rs:335` | `DROP TABLE IF EXISTS {t} CASCADE` | `t` = 카탈로그의 `storage_table` (2차) |
| `catalog/types.rs:414-421` | `CREATE TABLE og_data.n_<tid>` | D-06 |
| `catalog/types.rs:429-430` | `CREATE INDEX ON … (src|dst)` | D-06 |
| `catalog/types.rs:482-488` | `ALTER TABLE … ADD COLUMN {col} {dtype}` | **F-06** |
| `catalog/types.rs:550-554` | `ALTER TABLE … ADD COLUMN / SET NOT NULL` | D-05 + D-07 (정상 경로) |
| `catalog/types.rs:561-567` | `UPDATE … SET {col} = (__ext ->> '<prop>')::{dtype}` | D-05 + `sql_str` 3회 |
| `catalog/types.rs:573-575` | `CREATE UNIQUE INDEX uq_{sub}_{col}` | D-05 |
| `catalog/types.rs:610` | `CREATE INDEX ix_{sub}_{col}` | D-05 |
| `catalog/types.rs:702` | `DROP TABLE IF EXISTS {table} CASCADE` | 2차 |
| `cypher/views.rs:114` | `NULL::{dt} AS {col}` | **F-06** |
| `cypher/views.rs:118, 132, 135` | `CREATE OR REPLACE VIEW …` | D-06 + 2차 |
| `cypher/views.rs:175` | `DROP VIEW IF EXISTS {v} CASCADE` | `pg_class` 조회 결과 |
| `cypher/mod.rs:684` | `EXPLAIN ({opts}) {sql}` | `opts` 상수 2종, `sql` 컴파일 산출물 |
| `cypher/compile.rs` 전역 | 컴파일된 SELECT | D-01·D-05·D-06·D-10 |
| `storage/mod.rs:139` | `ALTER TABLE … ALTER COLUMN {col} TYPE text` | D-05 (단 [`05`](05_process_safety.md) DoS) |
| `storage/mod.rs:216, 212-214` | `({param}->>{lit})::{dtype}` | **F-05** |
| `storage/mod.rs:277-281, 306, 320-323, 344` | `INSERT/UPDATE {table} …` | D-06 + D-01 |
| `storage/mod.rs:374, 520` | `DELETE FROM {table} WHERE id = $1` | 2차 (`storage_table`) |
| `storage/traverse.rs:94-97, 244-247` | `SELECT … FROM og_data.og_adj WHERE …` | D-08 |
| `interop/mod.rs:24, 26, 27-29` | RLS DDL | **F-02** |
| `interop/mod.rs:74-101` | 매핑 뷰 DDL | **F-03** |
| `interop/mod.rs:139-146, 156-159` | 물질화 DDL | 2차 (`storage_table`) |
| `vector/mod.rs:58-61` | `CREATE INDEX … USING hnsw ({col} {opclass})` | D-05 + `metric_op` 화이트리스트(`:19-26`) |
| `vector/mod.rs:116, 126-132` | 벡터 검색 SELECT | **F-01** |
| `vector/mod.rs:180-186, 263-286, 429-431` | 유사/하이브리드/정확 검색 | D-05 + i32/f64 보간 |
| `vector/mod.rs:335-341` | 스테일 임베딩 조회 | **F-07** |
| `vector/mod.rs:372-378` | 임베딩 상태 갱신 | D-05 |
| `agent/mod.rs:427, 430, 438` | `SET <guc> = <number>` | D-11 |
| `agent/mod.rs:455-459` | `CREATE OR REPLACE TRIGGER og_hist_{sub} …` | D-06 (단 §8 search_path) |
| `agent/mod.rs:357` | `EXPLAIN (FORMAT JSON) {sql}` | 컴파일 산출물 |
| `compat/ddl.rs:160-165, 263-267` | 인덱스 DDL | D-09 + D-05 |
| `compat/procs.rs:189-196, 228-235` | 프로시저 FROM 절 | `sql_str` 인용(`:191-193`) |
| `typeql/schema.rs:231-234, 273-277, 538-543` | TypeQL 저장소 DDL | D-06 + `value_type_sql` 화이트리스트 |
| `typeql/write.rs:202, 242, 257, 318-328, 342-350, 400, 545, 599` | TypeQL 쓰기 | D-06 + `lit_str`(`typeql/compile.rs:616`) |
| `typeql/mod.rs:213-347` | TypeQL 파이프라인 | `quote_ident` + `Limit(i64)`/`Offset(i64)`(`typeql/ast.rs:26-27`) |
| `access.sql:220, 249` | plpgsql `EXECUTE format('… %s …')` | **F-04** |

---

## 8. 부수 발견 — `search_path` 미고정

어떤 함수도 `SET search_path`를 선언하지 않는다(`engine/sql/access.sql` 전체,
pgrx의 `#[pg_extern]`도 마찬가지). 그리고 컴파일러가 뱉는 SQL은 확장 함수를
**스키마 한정 없이** 부른다:

| 호출 | 위치 |
|---|---|
| `og_is_subtype(...)` | `engine/src/cypher/compile.rs:745` |
| `og_node_json(...)` | `engine/src/cypher/compile.rs:991, 1013, 1111` |
| `og_type_name(...)` | `engine/src/cypher/compile.rs:1101, 1119` |
| `og_reach(...)` / `og_vlp(...)` | `engine/src/cypher/compile.rs:870, 872` |
| `og_make_id(...)` | `engine/src/interop/mod.rs:75` |
| `og_capture_history()` (트리거 바인딩) | `engine/src/agent/mod.rs:458` |

**함의**: 확장이 `public` 스키마에 설치되고 `public`에 `CREATE` 권한이 남아
있으면(PostgreSQL 15 이전 기본값), 공격자가 동명 함수를 만들어 `search_path`
앞쪽에 두고 관리자가 `og_cypher`를 실행하도록 유인하는 CVE-2018-1058 계열
공격이 성립한다. 모든 함수가 호출자 권한으로 도는 덕에 자기 권한 상승은
아니지만, **관리자 세션을 표적으로 하는 상승은 가능하다.**

`og_enable_history`의 `CREATE TRIGGER … EXECUTE FUNCTION og_capture_history()`
(`agent/mod.rs:458`)는 특히 위험하다 — 트리거는 생성 시점의 함수 OID를 고정하므로,
잘못 바인딩되면 그 뒤 모든 쓰기가 공격자 함수를 거친다.

---

## 9. `ensure_alias_view`의 파괴적 부작용

```rust
// engine/src/catalog/types.rs:89-98
pub fn ensure_alias_view(tid: i32, name: &str, table: &str) {
    let view = alias_view_name(name);
    // A type name is user input; quoting is what makes that safe here.
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {view}"));
```

인용은 올바르지만(`alias_view_name:104-106`), 결과적으로
`DROP VIEW IF EXISTS og_data."<사용자가 정한 타입 이름>"` 이 실행된다.
`og_data` 스키마에 이름이 겹치는 뷰가 있으면 **그 뷰가 삭제된다.**
Cypher 쓰기 경로는 새 라벨을 자동 생성하므로
(`engine/src/catalog/types.rs:210-231` `resolve_or_create_label_set`),
`CREATE (n:v_5)` 같은 질의가 생성 뷰를 지울 수 있다.
반환값이 `let _ =` 로 버려져 실패도 감지되지 않는다.

---

## Forbidden (금지)

- **`og_vector_search`의 `filter`, `og_enable_rls`의 `policy_expr`,
  `og_map_table`의 `source_table` / `id_column` / `property_map` 값에
  최종 사용자 입력을 전달하지 말 것.** 이들은 원시 SQL을 받는다(F-01~F-03).
- **`og_catalog.type` / `og_catalog.property` / `og_catalog.embedding` 에
  애플리케이션 역할의 `INSERT`/`UPDATE` 권한을 부여하지 말 것** (F-04~F-07).
- **새 동적 SQL에서 `format!("'{x}'")` 형태를 쓰지 말 것.** 반드시
  `compile::sql_str(x)` 또는 `typeql::compile::lit_str(x)`를 경유할 것 (F-07이 그 반례).
- **식별자를 `format!` 로 직접 넣지 말 것.** `types::column_name`,
  `types::node_table`, `compile::quote_ident` 중 하나를 반드시 경유할 것.
- **`extension_sql_file`의 plpgsql에서 `format('%s', <카탈로그 값>)` 을 쓰지 말 것.**
  `%I` 또는 `%I.%I`를 쓸 것 (F-04).
- **확장을 `public` 스키마에 설치한 상태로 `public`의 `CREATE` 권한을
  `PUBLIC`에 남겨두지 말 것** (§8).

## Required (필수)

- 새 `#[pg_extern]`을 추가하면 §7 표에 한 행을 추가하고, "안전 근거" 열을
  D-01~D-12 중 하나로 채우거나 §3/§4의 결함으로 등록할 것.
- 새 프로퍼티 이름을 받는 경로를 추가하면 §5의 충돌 표를 재검토할 것.
- `access.sql`에 plpgsql 함수를 추가하면 `SET search_path = og_catalog, og_data, pg_catalog`
  를 함께 선언할 것 (§8).
- 회귀 테스트: `engine/tests/sql/` 에 (a) 작은따옴표를 포함한 프로퍼티 이름,
  (b) `a b` / `a-b` 충돌, (c) 시스템 뷰 이름과 겹치는 라벨 — 세 케이스를 추가할 것.

<!-- affects: security, backend, data, api -->
<!-- requires-update: 07_security/09_improvements_security.md, 02_api/02_cypher_api.md -->
