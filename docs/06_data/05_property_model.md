# 05. 프로퍼티 모델 — 실컬럼과 `__ext`

> **이 문서가 답하는 질문**
> - 어떤 프로퍼티가 실제 컬럼이 되고 어떤 것이 `__ext` jsonb에 남는가?
> - 쓰기 시점 컬럼 승격은 정확히 어떤 조건에서 일어나는가?
> - text로의 확장(widening)은 왜 단방향인가? 무엇을 확장하지 않는가?
> - `og_data."TypeName"` 별칭 뷰와 `og_data.v_<id>` 합집합 뷰는 각각 무엇인가?

**정본**: [`engine/src/storage/mod.rs:36-240`](../../engine/src/storage/mod.rs),
[`engine/src/catalog/types.rs:508-614`](../../engine/src/catalog/types.rs),
[`engine/src/cypher/views.rs`](../../engine/src/cypher/views.rs) (177줄).

---

## 결정 — 프로퍼티는 jsonb 블롭이 아니라 실제 컬럼이다

타입 테이블의 기본 형태:
```sql
-- entity
CREATE TABLE og_data.n_<tid> (id int8 PRIMARY KEY, __ext jsonb);
-- relation
CREATE TABLE og_data.e_<tid> (id int8 PRIMARY KEY, src int8 NOT NULL,
                              dst int8 NOT NULL, __ext jsonb);
```
(`engine/src/catalog/types.rs:412-420`)

선언된 프로퍼티마다 `ALTER TABLE ... ADD COLUMN p_<name> <type>`가 붙는다
(`engine/src/catalog/types.rs:550`).

**얻는 것** (`engine/src/catalog/types.rs:3-6`):
- 컬럼 통계 → 플래너가 선택도를 안다
- B-tree / HNSW / GIN 인덱스를 그냥 걸 수 있다
- MVCC와 RLS가 컬럼 단위로 그대로 적용된다
- 값의 타입이 보존된다 (`->>`가 모든 것을 text로 만들지 않는다)

**대가**: 타입마다 물리 테이블이 하나씩 생긴다. 타입 1,000개면 테이블 1,000개다.

---

## 사실 — 물리 컬럼 이름 규칙

```rust
pub fn column_name(prop: &str) -> String {
    let mut s = String::with_capacity(prop.len() + 2);
    s.push_str("p_");
    for c in prop.chars() {
        if c.is_ascii_alphanumeric() || c == '_' { s.push(c.to_ascii_lowercase()); }
        else if c.is_alphanumeric() { s.extend(c.to_lowercase()); }
        else { s.push('_'); }
    }
    s
}
```
(`engine/src/catalog/types.rs:53-66`)

| 프로퍼티 이름 | 컬럼 이름 |
|---|---|
| `title` | `p_title` |
| `Title` | `p_title` |
| `created-at` | `p_created_at` |
| `이름` | `p_이름` |
| `val` (TypeQL 속성) | `val` — **예외**, `p_` 접두사 없음 (`engine/src/typeql/schema.rs:284`) |

**비ASCII 문자는 `_`로 접지 않고 보존한다.** 주석이 이유를 밝힌다 —
그러지 않으면 `이름`과 `용량`이 같은 컬럼(`p__`)으로 매핑되어 **두 프로퍼티가 조용히
합쳐진다**(`engine/src/catalog/types.rs:46-52`).

**여전히 남는 충돌**: `created-at`과 `created.at`과 `created at`은 전부 `p_created_at`이다.
`og_add_property`의 `UNIQUE (type_id, name)`은 **프로퍼티 이름**에만 걸려 있어
컬럼 이름 충돌은 `ADD COLUMN IF NOT EXISTS`로 조용히 흡수된다
(`engine/src/catalog/types.rs:550`). 두 프로퍼티가 한 컬럼을 공유하게 된다. → `DATA-17`

---

## 사실 — 허용되는 프로퍼티 타입

`og_add_property(..., data_type)`가 받는 문자열은 `map_data_type()`을 거친다
(`engine/src/catalog/types.rs:13-44`).

| 입력 | 컬럼 타입 |
|---|---|
| `string` / `text` / `str` | `text` |
| `int` / `integer` / `int4` | `int4` |
| `long` / `bigint` / `int8` | `int8` |
| `float` / `double` / `float8` | `float8` |
| `real` / `float4` | `float4` |
| `bool` / `boolean` | `bool` |
| `datetime` / `timestamptz` / `timestamp` | `timestamptz` |
| `date` | `date` |
| `uuid` | `uuid` |
| `numeric` / `decimal` | `numeric` |
| `json` / `jsonb` | `jsonb` |
| `text[]` / `string[]` | `text[]` |
| `int[]` / `bigint[]` | `int8[]` |
| `vector(N)` (N은 숫자) | `vector(N)` |
| 그 외 | `error!` — 선언 시점에 거부 |

**모르는 타입은 선언 시점에 거부한다.** 쓰기 시점에 실패하는 것보다 낫다는 판단이다
(`engine/src/catalog/types.rs:11-12`).

`timestamp`가 `timestamptz`로 매핑되는 점에 주의 — 타임존 없는 타임스탬프를 요구해도
타임존 있는 컬럼이 생긴다.

---

## 결정 — 쓰기 시점 컬럼 승격 (`declare_new_props`)

**문제**: Cypher 애플리케이션은 아무것도 선언하지 않는다. Neo4j에 선언할 스키마가 없기
때문이다. 승격이 없으면 Cypher 앱이 쓰는 모든 프로퍼티가 jsonb에 떨어지고,
거기서는 인덱스도 통계도 없으며 타입도 잃는다(`engine/src/storage/mod.rs:78-82`).

**해법**: 쓰기 경로가 프로퍼티를 보고 **그 자리에서 선언한다.**

```rust
fn declare_new_props(type_id: i32, props: &Value,
                     existing: &[(String, String, String)]) -> bool {
    for (key, value) in obj {
        let Some(want) = infer_column_type(value) else { continue };
        match existing.iter().find(|(n, _, _)| n == key) {
            None => { /* og_add_property(graph, type, key, want, false, false) */ }
            Some((_, col, dtype))
                if WIDENABLE.contains(&dtype.as_str()) && !type_accepts(dtype, want) =>
            { /* widen to text */ }
            _ => {}
        }
    }
}
```
(`engine/src/storage/mod.rs:87-158`)

### 어떤 값이 승격되는가

```rust
fn infer_column_type(v: &Value) -> Option<&'static str> {
    match v {
        Value::Bool(_)   => Some("bool"),
        Value::Number(n) => Some(if n.is_i64() || n.is_u64() { "int8" } else { "float8" }),
        Value::String(_) => Some("text"),
        _ => None,     // 배열·객체·null은 승격되지 않는다
    }
}
```
(`engine/src/storage/mod.rs:53-60`)

**스칼라만 승격된다.** 배열과 객체는 의도적으로 `__ext`에 남는다.
주석의 이유(`engine/src/storage/mod.rs:49-52`): 여기서 중요한 유일한 배열 프로퍼티는
`embedding`이고, 그것은 `og_add_embedding`이 `vector(N)`으로 선언한다.
먼저 jsonb로 선언해버리면 그 길이 사라진다.

**`null`은 승격되지 않는다.** `Value::Null`은 `None`으로 떨어지므로,
`CREATE (n:X {a: null})`은 `a` 컬럼을 만들지 않는다.

**호출 시점**: `plan_props()`가 매 쓰기마다 부른다(`engine/src/storage/mod.rs:180`).
승격이 일어났으면 프로퍼티 목록을 **다시 읽는다**(`engine/src/storage/mod.rs:180-200`).

---

## 결정 — text로의 확장은 단방향이다

```rust
/// Column types this module is allowed to widen. Exactly the ones it can
/// create by inference — anything else was declared deliberately.
const WIDENABLE: &[&str] = &["bool", "int8", "float8"];

fn type_accepts(declared: &str, wanted: &str) -> bool {
    if declared == wanted { return true; }
    matches!((declared, wanted),
             ("float8", "int8") | ("numeric", "int8") | ("numeric", "float8"))
}
```
(`engine/src/storage/mod.rs:62-73`)

### 규칙

| 현재 컬럼 타입 | 들어온 값이 원하는 타입 | 결과 |
|---|---|---|
| `int8` | `int8` | 그대로 |
| `float8` | `int8` | 그대로 (수치 확장은 허용) |
| `numeric` | `int8` / `float8` | 그대로 |
| `int8` | `text` | **`text`로 확장** |
| `int8` | `float8` | **`text`로 확장** (int8은 float8을 못 담으므로) |
| `bool` | 아무 다른 것 | **`text`로 확장** |
| `text` | 아무것 | 그대로 (`text`는 `WIDENABLE`에 없다 — 이미 최광의) |
| `vector(1536)` | `text` | **확장하지 않음** |
| `timestamptz` | `text` | **확장하지 않음** |
| `uuid`, `date`, `jsonb`, `text[]` … | 아무것 | **확장하지 않음** |

### 왜 단방향인가

두 가지 이유가 코드에 명시되어 있다.

1. **진동 방지** (`engine/src/storage/mod.rs:84-86`):
   Neo4j는 같은 프로퍼티가 노드마다 다른 타입을 갖는 것을 허용한다.
   `text`는 그 전부를 표현할 수 있는 유일한 컬럼 타입이고, 단방향이므로
   `int8 → text → int8 → text`로 왕복하지 않는다. 왕복은 매번 테이블 전체 재작성이다.

2. **의도적 선언 보호** (`engine/src/storage/mod.rs:121-126`):
   > "A property declared as `vector(1536)` or `timestamptz` was declared on purpose —
   > by `og_add_embedding`, or by an application that knows its own schema — and
   > turning it into text because one write disagreed would destroy that intent.
   > (2026-08-16: doing so broke the vector suite, which is what this guard is for.)"

   `WIDENABLE`이 정확히 `infer_column_type`이 만들 수 있는 세 타입인 이유가 이것이다.
   **엔진이 추측해서 만든 것만 엔진이 바꿀 수 있다.**

### 확장의 실제 비용

```rust
for sub in labeling::og_subtypes(type_id) {
    if let Some(table) = types::storage_table(sub) {
        types::drop_alias_view(&n);
        Spi::run(&format!("ALTER TABLE {table} ALTER COLUMN {col} TYPE text USING {col}::text"));
        types::ensure_alias_view(sub, &n, &table);
    }
}
UPDATE og_catalog.property SET data_type = 'text' WHERE type_id = ANY($1) AND name = $2
```
(`engine/src/storage/mod.rs:127-153`)

- **`ALTER COLUMN ... TYPE`은 테이블 전체를 재작성한다.** `ACCESS EXCLUSIVE` 락이며
  그 컬럼의 모든 인덱스를 재구축한다.
- **모든 서브타입 테이블에 대해 반복된다.**
- 별칭 뷰를 먼저 드롭해야 한다 — 뷰가 의존하는 컬럼은 `ALTER`가 거부하기 때문이다
  (`engine/src/storage/mod.rs:134-137`).
- **한 번의 `CREATE` 문이 이걸 촉발할 수 있다.** 1억 행짜리 타입에 `{count: "12"}`를
  쓰면 그 자리에서 전체 재작성이 시작된다. → `PERF-11`
- 실패해도 조용하다 — `let _ = Spi::run(...)`(`engine/src/storage/mod.rs:138`).
  ALTER가 실패하면 카탈로그만 `text`로 바뀌고 컬럼은 `int8`로 남는다. → `DATA-12`

---

## 사실 — `__ext`로 가는 것

```rust
fn ext_expr(plan: &PropPlan, param: &str) -> String {
    if plan.declared.is_empty() {
        format!("NULLIF({param}, '{{}}'::jsonb)")
    } else {
        format!("NULLIF({param} - ARRAY[{list}]::text[], '{{}}'::jsonb)")
    }
}
```
(`engine/src/storage/mod.rs:228-240`)

**`__ext` = 입력 jsonb에서 선언된 프로퍼티 이름을 전부 뺀 나머지.**
비면 `NULL`이 된다.

여기 남는 것:
- 배열 값 (`{tags: ["a","b"]}`)
- 객체 값 (`{meta: {...}}`)
- `null` 값
- 승격 시도가 실패한 프로퍼티 (`og_add_property`가 오류를 냈을 때 —
  `is_ok()` 확인 후 `changed`를 세팅하므로 실패는 조용히 넘어간다,
  `engine/src/storage/mod.rs:112-119`)

**갱신은 병합이다**:
```sql
UPDATE {table} SET ..., __ext = COALESCE(__ext,'{}'::jsonb) || COALESCE({ext},'{}'::jsonb)
WHERE id = $1
```
(`engine/src/storage/mod.rs:320-323`)
`||`이므로 기존 키는 유지되고 새 키가 덮어쓴다. **`__ext`에서 키를 지우는 경로는 없다.**

### `__ext`의 대가

| 잃는 것 | 왜 |
|---|---|
| 인덱스 | `__ext`에 대한 인덱스를 만드는 코드가 **없다** (확인: `engine/src/`에서 `gin` 매치는 `engine/src/compat/ddl.rs:265`의 전문 검색 표현식 인덱스뿐) |
| 통계 | jsonb 컬럼 하나의 통계는 개별 키의 선택도를 알려주지 않는다 |
| 타입 | `->>`는 text를 낸다. `->`로 jsonb 값을 살리는 경로가 따로 있다 (`engine/src/cypher/compile.rs:1002-1013`) |
| 지역성 | 큰 `__ext`는 TOAST로 나간다 (`__ext`에는 `SET STORAGE`가 없으므로 기본 `EXTENDED`) |

**필터 컴파일**:
```rust
match tid {
    // 타입이 알려진 경우: 선언 안 됐으면 확장 페이로드에만 있을 수 있다
    Some(_) => (format!("({alias}.__ext->>{})", sql_str(prop)), None),
    // 타입 미상 변수: 런타임에 카탈로그로 해석
    None => (format!("(og_node_json({alias}.id)->>{})", sql_str(prop)), None),
}
```
(`engine/src/cypher/compile.rs:986-992`)

두 번째 갈래가 특히 비싸다. `og_node_json()`은 `LANGUAGE plpgsql`이고
행마다 ① 레지스트리 조회 ② 동적 `EXECUTE`로 행 전체를 jsonb로 ③ `og_catalog.property`
조인 + `jsonb_object_agg`를 수행한다(`engine/sql/access.sql:208-235`).
**행당 서브쿼리 세 개**다. → `PERF-08`

---

## 사실 — 선언이 늦었을 때의 백필

`og_add_property`가 `__ext`에 이미 있는 값을 컬럼으로 끌어온다.

```sql
UPDATE {table} SET {col} = (__ext ->> 'prop')::{dtype}, __ext = __ext - 'prop'
 WHERE __ext ? 'prop'
```
(`engine/src/catalog/types.rs:561-570`)

주석의 이유(`engine/src/catalog/types.rs:556-560`): "write first, index later"는
스키마리스 그래프에서 오는 애플리케이션의 **정상적인 순서**이고, 백필이 없으면
그 위에 만든 인덱스가 아무것도 보지 못한다.

**성질**
- `WHERE __ext ? 'prop'` — 해당 키가 있는 행만 건드린다. 하지만 `__ext`에 인덱스가
  없으므로 **전체 스캔**이다.
- `::{dtype}` 캐스트가 실패하면 `error!`로 전체 실패한다 —
  `og_add_property`가 롤백된다(`engine/src/catalog/types.rs:568-570`). 이건 좋은 동작이다.
- 모든 서브타입 테이블에 대해 반복된다.
- `required = true`이면 기존 인스턴스가 있을 때 **선언 자체를 거부한다**
  (`engine/src/catalog/types.rs:524-537`). 백필 전략을 명시적으로 요구하는
  좋은 에러 메시지가 붙어 있다.

---

## 사실 — 두 종류의 생성 뷰

### 1. 별칭 뷰 `og_data."<TypeName>"`

```rust
pub fn ensure_alias_view(tid: i32, name: &str, table: &str) {
    let view = alias_view_name(name);                    // og_data."MeetingRoom"
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {view}"));
    if let Err(e) = Spi::run(&format!("CREATE VIEW {view} AS SELECT * FROM {table}")) {
        pgrx::log!("could not create the alias view for type {tid} ({name}): {e}");
    }
}
```
(`engine/src/catalog/types.rs:89-98`)

**목적**: 물리 테이블이 `n_45`라서 `\dt`가 `MeetingRoom`을 안 보여준다.
이 뷰가 사람과 BI 도구에게 이름을 돌려준다(`engine/src/catalog/types.rs:422-426`).

**성질**
- **실패해도 치명적이지 않다.** 이름이 기존 객체와 충돌해도 타입 생성은 성공한다
  (`engine/src/catalog/types.rs:94-97`).
- **컬럼이 늘 때마다 다시 만들어야 한다** — 뷰는 생성 시점의 컬럼 목록을 굳히기 때문이다.
  `og_add_property`가 마지막에 서브타입 전체의 별칭 뷰를 재생성한다
  (`engine/src/catalog/types.rs:591-598`).
- text 확장 시에도 드롭 → ALTER → 재생성이 필요하다(`engine/src/storage/mod.rs:134-143`).
- **읽기 전용 편의 기능이다.** 질의 컴파일러는 이 뷰를 쓰지 않는다.

### 2. 서브타입 합집합 뷰 `og_data.v_<tid>` / `ve_<tid>`

```sql
CREATE VIEW og_data.v_1 AS
  SELECT id, 1::int4 AS type_id, p_model, p_year, NULL::int4 AS p_range_km, __ext FROM og_data.n_1
  UNION ALL
  SELECT id, 4::int4,            p_model, p_year, p_range_km,             __ext FROM og_data.n_4
```
(`engine/src/cypher/views.rs:7-12`)

**이것이 Cypher 컴파일러가 실제로 읽는 관계다.**

만드는 규칙(`engine/src/cypher/views.rs:99-137`):
1. `view_properties(tid)` — 이 타입과 모든 후손이 가진 프로퍼티의 합집합 (이름 → (컬럼, 타입))
2. `concrete_tables(tid, is_edge)` — `storage_table`이 있고 이름 접두사가 맞는 후손들
3. 각 테이블에 대해: 자기가 가진 컬럼은 그대로, 없는 컬럼은 `NULL::<type> AS <col>`
4. 엣지면 `src`, `dst`도 포함
5. 후손이 하나도 없으면 형태만 맞는 빈 관계(`SELECT ... WHERE false`)

**핵심 성질**: 라벨 해석이 **컴파일 시점에** 끝난다.
후손 집합은 구간 인덱스 범위 스캔 한 번으로 나오고(`engine/src/cypher/views.rs:14-17`),
질의가 실행될 때는 라벨 술어가 이미 **플래너가 개별 비용을 매길 수 있는
구체 테이블 목록**으로 바뀌어 있다. 행당 라벨 해석 비용이 0이다.

**무효화**: `drop_all_views()`가 전부 드롭하고 `ensure_view()`가 필요할 때 다시 만든다.
"존재하면 신선하다"(`engine/src/cypher/views.rs:91-92`).

**비용**: 서브타입이 많으면 `UNION ALL` 분기가 많아진다. 각 분기는 개별 테이블 스캔이고,
프로퍼티 합집합이 넓으면 대부분의 컬럼이 `NULL::<type>` 리터럴이다.
50개 서브타입 × 100개 프로퍼티면 5,000개 컬럼 표현식짜리 뷰다.
> **미측정**: 이 규모에서의 계획 시간 영향은 측정하지 않았다.

---

## 사실 — 값이 SQL에 들어가는 방식

**모든 사용자 값은 jsonb 파라미터 하나로 바인딩된다.** SQL 텍스트에 보간되지 않는다
(`engine/src/storage/mod.rs:44-46`, spec 003 FR-026).

```rust
// 스칼라
format!("({param}->>{lit})::{dtype}")
// 배열
format!("(SELECT array_agg(x)::{dtype} FROM jsonb_array_elements_text({param}->{lit}) \
          AS t(x_raw), LATERAL (SELECT t.x_raw::{elem}) AS c(x))")
```
(`engine/src/storage/mod.rs:209-217`)

보간되는 것은 **컬럼 이름과 타입 이름**뿐이고, 둘 다 카탈로그에서 온 값이다.
프로퍼티 이름은 `quote_json_key()`로 작은따옴표를 이스케이프한다
(`engine/src/storage/mod.rs:224-226`).

> **예외**: TypeQL 속성 값은 이 규칙을 따르지 않는다.
> `intern_attribute`가 값을 SQL 리터럴로 만들어 문자열에 넣는다
> (`engine/src/typeql/write.rs:242-245, 260-263`). 이스케이프는
> `typed_literal` 한 곳에 모여 있다(`engine/src/typeql/write.rs:647-648`).
> 상세와 영향은 [`06_role_and_relation_model.md`](06_role_and_relation_model.md).

---

## 금지 / 필수

**금지**
- 대량 데이터가 이미 들어 있는 타입에 **타입이 흔들리는 프로퍼티**를 쓰는 것.
  한 번의 `CREATE`가 `ALTER TABLE ... TYPE text` 전체 재작성을 촉발한다.
- `__ext`에 든 프로퍼티로 필터링하는 질의를 뜨거운 경로에 두는 것. 인덱스가 없다.
- 타입 미상 변수(`MATCH (n) WHERE n.foo = ...`)로 프로퍼티를 읽는 것.
  `og_node_json()`이 행마다 세 개의 서브질의를 돈다.
- `og_data."<TypeName>"` 별칭 뷰에 `INSERT`/`UPDATE`하는 것.
  단순 뷰라 PostgreSQL이 자동 갱신 가능하게 만들지만, 그러면 `og_node` 레지스트리와
  `og_adj`가 어긋난다.
- 프로퍼티 이름에서 `-`, `.`, 공백만 다른 두 이름을 같은 타입에 쓰는 것 (컬럼이 합쳐진다).

**필수**
- **성능이 중요한 프로퍼티는 데이터를 넣기 전에 `og_add_property()`로 선언할 것.**
  나중에 선언해도 백필해주지만 그건 전체 테이블 UPDATE다.
- 선언 시 타입을 **가장 넓은 것으로** 고를 것. `int`가 아니라 `long`,
  값이 흔들릴 여지가 있으면 처음부터 `string`.
- 임베딩은 반드시 `og_add_embedding()`으로 선언할 것. 그래야 `vector(N)`이 되고,
  `WIDENABLE` 밖이라 실수로 text가 되지 않는다.
- 뷰 정의를 확인하고 싶으면:
  ```sql
  SELECT viewname, definition FROM pg_views
   WHERE schemaname = 'og_data' AND viewname LIKE 'v\_%';
  ```

---

<!-- affects: data, backend, api -->
<!-- requires-update: docs/06_data/09_query_access_paths.md, docs/06_data/10_improvements_data.md -->
