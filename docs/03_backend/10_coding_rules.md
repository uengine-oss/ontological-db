# 코딩 규칙 — 코드 리뷰 체크리스트

> **이 문서가 답하는 질문**
> - 이 코드베이스에서 무엇이 **금지**이고 무엇이 **필수**인가?
> - 각 규칙의 **왜**는 무엇이고, 근거는 어느 파일 몇 줄인가?
> - 리뷰에서 무엇을 확인해야 하는가?

**이 문서의 모든 규칙은 실제 코드에서 관찰된 것이다.** 일반론은 없다.
규칙마다 근거 `파일:라인`이 붙어 있고, 그 코드를 읽으면 규칙이 확인된다.

---

## 리뷰 체크리스트 (요약)

```
[ ] R-01  사용자 값이 SQL 텍스트에 보간되지 않았는가?
[ ] R-02  읽기 경로를 Rust 함수로 감싸지 않았는가?
[ ] R-03  access.sql 신규 함수가 LANGUAGE sql + STABLE + PARALLEL SAFE 인가?
[ ] R-04  집합 반환 함수에 ROWS 추정치를 명시했는가?
[ ] R-05  주석이 영어인가?
[ ] R-06  주석이 '무엇'이 아니라 '왜'를 말하는가?
[ ] R-07  엣지 쓰기가 양방향 인접 세그먼트를 함께 갱신하는가?
[ ] R-08  새 사용자 데이터 테이블이 pg_extension_config_dump 에 등록됐는가?
[ ] R-09  사용자 도달 가능한 실패가 unwrap() 이 아니라 error!(설명) 인가?
[ ] R-10  오류 메시지가 '무엇이 틀렸는지 + 무엇을 하면 되는지'를 담는가?
[ ] R-11  stable/immutable 함수가 DDL/DML을 하지 않는가?
[ ] R-12  타입 계층 판정에 재귀 CTE를 쓰지 않았는가?
[ ] R-13  SPI 결과를 클로저 밖으로 반출하지 않았는가?
[ ] R-14  트랜잭션 제어문(BEGIN/COMMIT/SAVEPOINT)을 쓰지 않았는가?
[ ] R-15  게이트웨이가 Cypher를 해석하지 않는가?
[ ] R-16  스키마를 바꾸는 경로가 bump_schema_version 을 부르는가?
[ ] R-17  다중도 판정(blind_expr/multiplicity_blind)을 넓히면서 테스트를 먼저 추가했는가?
[ ] R-18  새 예약어를 추가하면서 그것이 라벨/프로퍼티로 쓰일 수 있는지 확인했는가?
[ ] R-19  Spi::run 의 반환값을 무의미하게 버리지 않았는가?
[ ] R-20  새 순수 함수에 #[cfg(test)] 테스트를 추가했는가?
```

---

## A. SQL 생성과 주입

### R-01 【금지】 사용자 값을 SQL 문자열에 보간하는 것

**왜**: spec 003 FR-026. 주입 방지가 규약이 아니라 **구조**여야 한다.
파라미터가 딱 하나면 "여기만 안전하면 전부 안전하다"가 성립한다.

**근거**:
- `engine/src/cypher/compile.rs:18` — `pub const PARAM: &str = "$1";` (유일한 파라미터 자리)
- `engine/src/cypher/compile.rs:1156-1162` — `Expr::Param` → `($1 ->> 'name')::type`
- `engine/src/cypher/mod.rs:145-152` — 실행 시 jsonb 하나만 바인딩
- `engine/src/storage/mod.rs:44-46` — "All values are extracted from ONE bound jsonb parameter,
  so no user value is ever interpolated into SQL text (spec 003 FR-026)."
- `bolt/src/session.rs:544-545` — 게이트웨이도 같은 보장을 유지

**허용되는 보간** (값이 아닌 것):

| 보간 대상 | 안전 근거 |
|---|---|
| 테이블 이름 `og_data.n_<tid>` | `catalog/types.rs:68-74` — `tid: i32` |
| 컬럼 이름 `p_<prop>` | `catalog/types.rs:53-66` `column_name()` — 화이트리스트 치환 |
| 타입 id 배열 | `cypher/compile.rs:844-847` — 전부 `i32` |
| SQL 문자열 리터럴 | `cypher/compile.rs:1589-1591` `sql_str()` — `'` → `''` |
| SQL 식별자 | `cypher/compile.rs:1584-1586` `quote_ident()` — `"` → `""` + 항상 인용 |
| 방향 문자 `'o'`/`'i'`/`'b'` | `storage/traverse.rs:36-41` `check_dir()` — 3원소 집합 검증 후 |

`storage/traverse.rs:52-57`이 이 구분을 명시적으로 정당화한다:

> The direction is validated against a three-element set before it reaches here,
> so the format is not an injection path; the type ids still go in as a bound
> parameter, because a thousand-element list rewritten per call would be its own cost.

**현재 위반**: TypeQL 쓰기 경로가 속성 값을 SQL 리터럴로 만든다
(`typeql/write.rs:649-674` `typed_literal`, 사용처 `write.rs:242,257,321`).
이스케이프는 하고 있으나 규칙과 어긋난다 → `CODE-08`.

**리뷰 확인법**: `format!` 안에 `{}`가 들어가는 SQL을 보면,
그 자리에 들어가는 값의 출처를 역추적한다. 사용자 입력이면 거부.

---

### R-02 【금지】 읽기 경로를 Rust 함수로 감싸는 것

**왜**: Rust 집합 반환 함수는 인라인되지 않는다. 플래너가 순회 자체를 못 보면
이 프로젝트가 Apache AGE와 다르다는 근거가 사라진다. 헌법 원칙 II.

**근거**:
- `engine/src/storage/mod.rs:7-10`:
  > Read paths are deliberately **not** here. The Cypher compiler emits SQL that
  > touches `og_data.og_adj` directly so the PostgreSQL planner sees the whole
  > traversal — that is the difference between this design and a function-call
  > pipeline the optimiser cannot look into.
- `engine/sql/access.sql:4-8`:
  > A PL/pgSQL or C set-returning function would be an optimisation barrier, which
  > is precisely the mistake Constitution principle II forbids.
- 실제 산출물: `cypher/compile.rs:900-904`가 `og_data.og_adj`를 직접 스캔하는 LATERAL을 생성

**허용된 예외 1** — `og_reach` (`storage/traverse.rs:80-161`):
방문집합이 필요하고 SQL에 그런 게 없다. 대가를 치른다:
- `parallel_restricted`
- `access.sql:192-197`에서 `ROWS 100`으로 `og_vlp`와 비용을 맞춘다
- `compile.rs:20-33`이 SPI 셋업 비용을 명시적으로 손익분기 계산에 넣는다

**허용된 예외 2** — `og_node_json` / `og_edge_json` (`access.sql:209,238`):
`LANGUAGE plpgsql`이어야 한다. 스토리지 테이블 이름을 런타임에 결정해야 하기 때문
(`EXECUTE format(...)`). 대가는 인라인 불가 + `PARALLEL UNSAFE`다.

---

### R-03 【필수】 `access.sql`의 공개 함수는 `LANGUAGE sql` + `STABLE` + `PARALLEL SAFE`

**왜**: `access.sql:4-8` — 단순 SQL 함수는 호출 질의에 인라인되므로 플래너가
통계·조인 순서·병렬성을 그대로 쓴다.

**근거** — 현재 준수 현황:

| 함수 | 선언 | 라인 |
|---|---|---|
| `og_expand` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 50` | `access.sql:16` |
| `og_expand_batch` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 500` | `access.sql:31` |
| `og_subtype_ids` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 8` | `access.sql:45` |
| `og_nodes` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000` | `access.sql:55` |
| `og_edges` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000` | `access.sql:62` |
| `og_type_id` | `LANGUAGE sql STABLE PARALLEL SAFE` | `access.sql:72` |
| `og_vlp` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 100` | `access.sql:140` |
| `og_reach_sql` | `LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000` | `access.sql:171` |
| `og_type_name` | `LANGUAGE sql STABLE PARALLEL SAFE` | `access.sql:204` |
| `og_prop` | `LANGUAGE sql STABLE` — **PARALLEL SAFE 없음** | `access.sql:268` |
| `og_node_json` / `og_edge_json` | `LANGUAGE plpgsql STABLE` | `access.sql:209,238` |
| `og_capture_history` | `LANGUAGE plpgsql` (트리거) | `access.sql:274` |

`og_prop`에 `PARALLEL SAFE`가 없는 건 **올바르다** — 본문이 `og_node_json()`을 부르는데
그것이 plpgsql이라 병렬 안전하지 않기 때문이다. 규칙을 어긴 게 아니라 규칙을 정확히 적용한 결과다.

**리뷰 확인법**: 새 SQL 함수가 다른 함수를 부르면, 그 함수의 병렬 안전성을 확인한다.

---

### R-04 【필수】 집합 반환 함수에 `ROWS` 추정치를 명시

**왜**: PostgreSQL 기본 추정치는 1000이다. 같은 질문에 답하는 두 함수가
10배 다른 비용을 받으면 플래너가 다른 조인 순서를 고른다.

**근거** — `engine/sql/access.sql:192-197`:

> `og_reach` is written in Rust, and pgrx gives every set-returning function
> PostgreSQL's default guess of 1000 rows. `og_vlp` declares 100. Two functions
> that answer the same question must not be costed an order of magnitude apart for
> a reason that has nothing to do with either — the planner would pick different
> join orders for the two and the comparison would measure the guess.
>
> `ALTER FUNCTION og_reach(int8, int4[], "char", int4, int4) ROWS 100;`

**리뷰 확인법**: 새 `TableIterator` / `SetOfIterator` 반환 `#[pg_extern]`을 추가하면
`access.sql`에 `ALTER FUNCTION … ROWS n`을 함께 추가했는지 본다.

---

### R-05 【필수】 코드 주석은 영어

**왜**: 프로젝트 규약. 소스 전체가 일관되게 영어이며, 한국어 식별자·데이터는
문자열/테스트 데이터로만 등장한다.

**근거**: `engine/src/**/*.rs` 전체. 예 —
`cypher/lexer.rs:129-132`("A graph whose classes are named in Korean — `(:회의실)` —"),
`catalog/types.rs:48-52`(한국어 프로퍼티 이름 설명), `04_neo4j_compat.sql:12`(한국어 데이터).

한국어는 **문서**(`docs/`)와 **테스트 데이터**에서만 쓴다.

---

### R-06 【필수】 주석은 '무엇'이 아니라 '왜'를 말한다

**왜**: 이 코드베이스의 주석은 사실상 **설계 결정 기록(ADR)**으로 기능한다.
측정값과 실패 이력이 주석에 남아 있어서, 나중에 같은 실수를 반복하지 않게 한다.

**근거** — 모범 사례:

| 위치 | 무엇을 기록했는가 |
|---|---|
| `cypher/compile.rs:36-41` | `WALKS = 512`가 유도값이 아니라 측정 결과이고, 두 실패 모드가 비대칭이라는 것 |
| `cypher/compile.rs:60-67` | 폐기된 이전 규칙과 그것이 틀린 구체적 사례 (1000×1000 격자, 3.83 ms vs 0.30 ms) |
| `storage/mod.rs:121-126` | 벡터 컬럼을 확장 대상에서 뺀 이유 + **날짜(2026-08-16)와 깨진 스위트** |
| `storage/traverse.rs:100-106` | 루프 안에서 재계획하면 10배 느려진 측정 |
| `compat/ddl.rs:310-323` | `IS NOT NULL`을 컬럼 NOT NULL로 강제하지 않는 두 가지 이유 |
| `cypher/compile.rs:130-136` | OPTIONAL MATCH 술어를 `WHERE`에 두면 안 되는 이유 |
| `bolt/src/session.rs:256-264` | summary `type` 한 필드가 왜 중요한지 (Neo4j MCP 서버가 여기 걸려 있음) |

**리뷰 확인법**: 새 주석이 코드를 다시 말하고 있으면(`// increment counter`) 지적한다.
비자명한 결정에는 반드시 이유가 붙어야 한다.

---

## B. 스토리지 정합성

### R-07 【필수】 엣지 쓰기는 양방향 인접 세그먼트를 함께 갱신

**왜**: spec 001 FR-012. 한쪽만 갱신하면 `MATCH (a)<-[:R]-(b)`가 조용히 답을 잃는다.

**근거**:
- `engine/src/storage/mod.rs:444-446`:
  ```rust
  // Both adjacency directions, same transaction — spec 001 FR-012.
  adjacency::append(src, tid, 'o', dst, eid);
  adjacency::append(dst, tid, 'i', src, eid);
  ```
- 삭제: `storage/mod.rs:516-517`
- TypeQL 쪽도 동일: `typeql/write.rs:404-405`(생성), `write.rs:542-543`(삭제)
- 검증: `og_check_integrity()`의 `missing_adjacency` 검사 (`storage/stats.rs`)

**하위 규칙**: `nbr`과 `eid` 배열은 **항상 같은 인덱스로** 조작한다
(`storage/adjacency.rs:51-60`). 정렬이 깨지면 `unnest(nbr, eid)`가 잘못된 쌍을 만든다.

---

### R-08 【필수】 새 사용자 데이터 테이블은 `pg_extension_config_dump`에 등록

**왜**: `CREATE EXTENSION` 스크립트가 만든 테이블은 확장 소유이고,
`pg_dump`는 그것에 대해 `CREATE EXTENSION`만 내보낸다 — **내용은 건너뛴다.**

**근거** — `engine/sql/bootstrap.sql:381-389`:

> Every relation below holds user data, so each one has to be registered as
> configuration data or a dump would silently restore an empty graph.

등록 목록: `bootstrap.sql:390-427` (테이블 26개 + 시퀀스 11개).
`og_catalog.setting`은 부트스트랩이 심는 4개 키를 제외하는 `WHERE` 절과 함께 등록된다
(`bootstrap.sql:412-414`).

런타임에 생기는 `og_data.n_*` / `e_*` / `a_*`는 확장 소유가 아니므로 자동으로 덤프된다
(`bootstrap.sql:387-388`).

**검증**: `tests/run.sh:39-71`의 백업 왕복 게이트.

---

### R-12 【금지】 타입 계층 판정에 재귀 CTE를 쓰는 것

**왜**: spec 002 FR-010이 금지하고 SC-003이 EXPLAIN 출력을 검사해 강제한다.
헌법 원칙 IV.

**근거**:
- `engine/src/catalog/labeling.rs:1-16` — 구간(nested-set) 라벨의 존재 이유
- `engine/sql/access.sql:40-51` — `og_subtype_ids`가 재귀 없이 범위 비교 하나로 답한다
  > note the complete absence of recursion
- `catalog/labeling.rs:233-244` `og_is_subtype`:
  ```sql
  SELECT EXISTS (SELECT 1 FROM og_catalog.type_label d, og_catalog.type_label a
                  WHERE d.type_id = $1 AND a.type_id = $2 AND a.graph_id = d.graph_id
                    AND d.lft >= a.lft AND d.rgt <= a.rgt)
  ```

**허용된 재귀** (계층이 아닌 것):

| 위치 | 무엇에 대한 재귀 |
|---|---|
| `access.sql:138-156` `og_vlp` | **데이터**(그래프) 순회 — `access.sql:135-136`가 명시적으로 구분 |
| `access.sql:169-187` `og_reach_sql` | 같음 |
| `catalog/types.rs:459-465` `copy_inherited_properties` | 프로퍼티 상속 (DDL 시점 1회) |
| `typeql/compile.rs:645-650` `role_with_specialisations` | **롤** 특수화 사슬 (타입 계층 아님) |

**리뷰 확인법**: `WITH RECURSIVE`를 보면 그것이 `type_parent`를 걷는지 확인한다.
걷는다면 구간 라벨로 바꿀 수 있는지 검토한다.

---

### R-16 【필수】 스키마를 바꾸는 경로는 `bump_schema_version`을 호출

**왜**: 생성된 타입 뷰(`og_data.v_*`, `ve_*`)가 자손 집합을 인코딩하고 있다.
스키마가 바뀌면 그 뷰가 거짓이 된다.

**근거** — `engine/src/catalog/labeling.rs:172-182`:

```rust
pub fn bump_schema_version(graph_id: i32, description: &str) {
    // Generated per-type union views encode the descendant set, so any schema
    // change invalidates them (spec 003 / cypher::views).
    crate::cypher::views::drop_all_views();
    Spi::run_with_args("INSERT INTO og_catalog.schema_version …", …)
}
```

호출처: `types.rs:317`(그래프 생성), `types.rs:599`(프로퍼티 추가), `types.rs:653`(롤),
`types.rs:681`(규칙), `labeling.rs:169`(재라벨링), `cypher/mod.rs:572`(라벨 이름 변경),
`typeql/schema.rs:126`(define).

에이전트가 스키마를 캐시하고 이 버전을 무효화 키로 쓴다 (`bootstrap.sql:173-175`, spec 008 FR-005).

> **주의**: 이 규칙은 지켜지고 있지만, `PLAN_CACHE`는 이 신호를 **받지 않는다** → `CODE-01`.

---

## C. 함수 선언

### R-11 【금지】 `stable` / `immutable` 함수에서 DDL·DML을 하는 것

**왜**: PostgreSQL의 `STABLE`은 "데이터베이스를 수정하지 않는다"는 계약이다.
어기면 읽기 전용 트랜잭션·스탠바이·병렬 워커에서 실패한다.

**현재 위반 2건**:

| 함수 | 선언 | 실제로 하는 일 |
|---|---|---|
| `og_cypher_sql` | `#[pg_extern(stable)]` `cypher/mod.rs:74` | `views::ensure_view` → `CREATE OR REPLACE VIEW` (`views.rs:135`) |
| `og_typeql_sql` | `#[pg_extern(stable)]` `typeql/mod.rs:82` | `Compiler::new` → `ensure_has_type` → `INSERT` + `CREATE TABLE` (`typeql/schema.rs:526-552`) |

→ `CODE-02`.

**올바른 예**: `og_cypher`는 속성 없이 선언되어 기본 `VOLATILE`이다 (`cypher/mod.rs:83`).
`og_cypher_stats`는 명시적으로 `volatile, parallel_unsafe` (`cypher/mod.rs:117`) —
백엔드-로컬 상태를 읽으므로 정확한 선택이다.

---

### R-19 【금지】 `Spi::run`의 결과를 이유 없이 버리는 것

**왜**: 실패가 조용히 지나가면 카탈로그와 물리 구조가 어긋난다.

**현재 `let _ = Spi::run(...)` 사용처 4곳**:

| 위치 | 판정 |
|---|---|
| `catalog/types.rs:92` `DROP VIEW IF EXISTS` | **허용** — `IF EXISTS`이고 뒤에 재생성 |
| `catalog/types.rs:101` `DROP VIEW IF EXISTS` | **허용** — 같음 |
| `storage/mod.rs:138` `ALTER TABLE … TYPE text` | ⚠️ **위험** — 실패해도 카탈로그는 갱신됨 |
| `storage/mod.rs:147` `UPDATE og_catalog.property … data_type='text'` | ⚠️ **위험** — 같은 짝 |

`.ok()`로 버리는 곳 26군데 중 판정이 갈리는 것:

| 위치 | 판정 |
|---|---|
| `catalog/types.rs:704-706` 타입 삭제 시 `DELETE FROM og_node/og_edge/og_adj` | ⚠️ 실패 시 고아 데이터 |
| `storage/mod.rs:379` `DELETE FROM og_adj WHERE src = $1` | ⚠️ 실패 시 dangling adjacency |
| `storage/mod.rs:525` `DELETE FROM og_role_player` | ⚠️ 같음 |
| `typeql/write.rs:545-547, 596-602` 삭제 경로 전체 | ⚠️ 같음 |
| `cypher/views.rs:175` `DROP VIEW IF EXISTS … CASCADE` | 허용 |
| `agent/mod.rs:427-438` `SET statement_timeout` 등 | 허용(설정은 best-effort) |
| `vector/mod.rs:428,440` `SET LOCAL enable_indexscan` | 허용 |
| `interop/mod.rs:26,97,141,144` `DROP … IF EXISTS` | 허용 |
| `cypher/mod.rs:134` / `typeql/mod.rs:127` 감사 INSERT | **의도적 허용** — 감사가 질의를 막으면 안 됨 |

→ `CODE-07`.

**리뷰 확인법**: `let _ =` 또는 `.ok();`를 보면 "이게 실패하면 어떤 불변식이 깨지는가"를 묻는다.
답이 있으면 `.unwrap_or_else(|e| error!(...))`로 바꾼다.

---

## D. 오류

### R-09 【금지】 사용자 도달 가능한 실패를 `unwrap()`으로 처리하는 것

**왜**: `unwrap()`은 `called Option::unwrap() on a None value`를 낸다.
사용자는 자기가 무엇을 잘못했는지 알 수 없다.

**올바른 패턴** — `catalog/types.rs:112-119`:

```rust
pub fn graph_id(name: &str) -> i32 {
    crate::spiu::one::<i32>("SELECT graph_id FROM og_catalog.graph WHERE name = $1", &[name.into()])
        .expect("graph lookup failed")                                  // SPI 실패 = 내부 오류
        .unwrap_or_else(|| error!("graph '{name}' does not exist"))     // 결과 없음 = 사용자 오류
}
```

**두 층을 반드시 구분한다**:
- `Result` 층(`.expect`) — SPI 자체가 실패. 진짜 내부 오류.
- `Option` 층(`.unwrap_or_else(|| error!(…))`) — 결과 없음. 사용자에게 설명.

**허용되는 `expect`** — 내부 불변식:

```rust
// storage/traverse.rs:272
let pos = |id: i64| ids.binary_search(&id).expect("id present by construction") as u32;
```

`expect` 문자열이 **왜 실패할 수 없는지**를 말한다.

**현재 위반 예**: `catalog/labeling.rs:44` `row.get(1).unwrap().unwrap()` — 이중 언랩.
전체 202건 중 위험한 것의 목록은 `CODE-25`.

---

### R-10 【필수】 오류 메시지는 '무엇이 틀렸는지 + 무엇을 하면 되는지'

**왜**: spec 003 FR-008, spec 008. `agent/mod.rs:1-8`:

> the entity writing Cypher against this database is increasingly a language model …
> the database owes it three things: an accurate machine-readable schema,
> **errors that carry their own correction**, and limits that stop a bad query
> before it stops the server.

**모범 사례**:

```rust
// catalog/types.rs:532-536
error!("cannot add required property '{prop}' to '{type_name}': {n} existing instance(s) \
        would violate it. add it as optional, backfill, then tighten.");

// cypher/compile.rs:1558-1566  — 지원 목록 전체를 메시지에
"unknown function '{other}'. supported: count, sum, avg, min, max, collect, id, elementId, …"

// compat/procs.rs:154-159  — 프로시저도 동일
"procedure '{other}' is not available. supported: db.index.vector.queryNodes, …"
```

**이름 불일치에는 편집 거리 힌트를 붙인다** (`catalog/types.rs:236-261` `nearest_type_names`):

```rust
error!("type '{name}' does not exist. did you mean: {}", hint.join(", "));
```

**리뷰 확인법**: 새 `error!`를 보면 "이 메시지를 받은 LLM이 다음에 무엇을 시도할지
알 수 있는가"를 묻는다.

---

## E. 트랜잭션과 SPI

### R-13 【금지】 SPI 클라이언트/테이블을 클로저 밖으로 반출하는 것

**왜**: `SpiClient`의 수명이 `Spi::connect` 클로저에 묶여 있다. 반출은 컴파일되지 않거나
안전하지 않다.

**규약**: 항상 클로저 안에서 `Vec` / 스칼라로 물질화한 뒤 반환한다.

**근거** — 이 패턴이 전 코드에 일관된다:
`cypher/views.rs:36-52`, `storage/mod.rs:161-177`, `storage/traverse.rs:107-158`,
`catalog/labeling.rs:35-68`, `typeql/compile.rs:642-657`.

헬퍼는 `engine/src/spiu.rs`의 셋만 쓴다: `one` / `two` / `one_mut`.
존재 이유는 `spiu.rs:3-6` — `Spi::get_one_with_args`가 "결과 없음"과 "고장남"을 혼동하기 때문.

---

### R-14 【금지】 트랜잭션 제어문을 실행하는 것

**왜**: 확장은 **호출자의 트랜잭션 안에서** 산다. `BEGIN`/`COMMIT`/`ROLLBACK`/`SAVEPOINT`를
실행하면 호출자의 경계를 깨뜨린다.

**근거**: 전 소스에서 해당 문장 **0건**. Bolt 게이트웨이만이 예외이며,
그것은 확장 밖의 별도 프로세스다 (`bolt/src/session.rs:208-219`).

---

## F. 언어 표면

### R-15 【금지】 게이트웨이가 Cypher를 해석하는 것

**왜**: 질의 경로는 하나여야 한다. 두 번째 파서는 반드시 첫 번째와 어긋난다.

**근거** — `bolt/src/main.rs:3-5`, `bolt/src/session.rs:438-441`:

> Does this query write? Answered by the engine's own parser, never by a keyword
> scan here — `CREATE` inside a string literal is not a write, and only the parser
> knows that.

게이트웨이가 하는 유일한 텍스트 처리는 `split_plan_prefix`(`session.rs:471-482`) —
선행 `EXPLAIN`/`PROFILE` 제거뿐이다.

---

### R-17 【금지】 다중도 판정을 테스트 없이 넓히는 것

**왜**: `cypher/compile.rs:327-328`:

> The test is deliberately narrow, because being wrong here changes **answers**
> rather than timings.

**근거**: `multiplicity_blind`(`compile.rs:339-349`), `blind_expr`(`compile.rs:82-100`).
고정 테스트: `engine/tests/sql/05_reachability.sql:72-96` (6개 케이스 + 손익분기 1개).

**리뷰 확인법**: 이 두 함수를 건드린 diff를 보면, 같은 커밋에
`05_reachability.sql` 변경이 있는지 확인한다. 없으면 거부.

---

### R-18 【금지】 예약어를 늘리면서 그 단어가 이름으로 쓰일 수 있는지 확인하지 않는 것

**왜** — `cypher/lexer.rs:24-27`:

> Deliberately absent: INDEX, CONSTRAINT, FOR, REQUIRE, UNIQUE, OPTIONS, EACH, IF,
> VECTOR, FULLTEXT, RANGE, TEXT, POINT, KEY. They appear only in DDL, where the parser
> matches them by spelling — reserving them would stop anyone from having a property
> called `text` or a label called `Range`.

DDL 전용 단어는 `at_word` / `eat_word` / `expect_word`(`parser.rs:253-280`)로 철자 매칭한다.

키워드가 이름 자리에 오는 것도 지원해야 한다 (`parser.rs:103-118` `name()` — `Token.raw` 사용).
`[r:CONTAINS]`, `(n:Order)`가 그 예다.

TypeQL 렉서는 아예 키워드 토큰이 없다 (`typeql/lexer.rs:15-27`) — 전부 `Ident`이고
파서가 위치로 판단한다.

---

## G. 테스트

### R-20 【필수】 새 순수 함수에 `#[cfg(test)]` 테스트를 추가

**왜**: 순수 함수는 DB도 pgrx 런타임도 필요 없다. 테스트하지 않을 이유가 없다.

**근거**: `engine/src/lib.rs:45-48` — 순수 Rust 단위는 모듈 옆에 두고 `cargo test`로 돈다.
현재 9개 파일 35개 테스트.

**현재 공백** (전부 순수 함수인데 테스트 없음):

| 함수 | 위치 |
|---|---|
| `blind_expr`, `multiplicity_blind`, `mentions_alias`, `quote_ident`, `sql_str` | `cypher/compile.rs` |
| `infer_column_type`, `type_accepts` | `storage/mod.rs:53,67` |
| `column_name`, `edit_distance`, `map_data_type` | `catalog/types.rs:53,264,13` |
| `pack`, `mark`, `flip`, `build_key` | `storage/traverse.rs:224,348,44,212` |
| `speaks`, `split_plan_prefix`, `to_bolt`, `to_json`, `record`, `Failure::from_pg` | `bolt/src/session.rs` |
| `apoc_type`, `fulltext_expr`, `sanitize`, `default_name` | `compat/meta.rs:24`, `compat/ddl.rs:272,280,170` |
| `typed_literal`, `json_literal` | `typeql/write.rs:649,676` |
| `value_type_sql`, `annotations_json` | `typeql/schema.rs:28,445` |
| `agg_sql` | `typeql/mod.rs:370` |

→ `CODE-29`.

---

## H. 명명 규약 (관찰된 것)

| 대상 | 규약 | 예 |
|---|---|---|
| 공개 SQL 함수 | `og_` 접두사 + snake_case | `og_cypher`, `og_add_property` |
| 내부 Rust 함수 (SQL 함수의 본체) | 같은 이름 + `_inner` | `create_node_inner`, `delete_edge_inner` |
| 노드 스토리지 테이블 | `og_data.n_<type_id>` | `catalog/types.rs:68-70` |
| 엣지 스토리지 테이블 | `og_data.e_<type_id>` | `catalog/types.rs:72-74` |
| 속성 스토리지 테이블 (TypeQL) | `og_data.a_<type_id>` | `typeql/schema.rs:48-50` |
| 노드 타입 뷰 | `og_data.v_<type_id>` | `cypher/views.rs:23-25` |
| 엣지 타입 뷰 | `og_data.ve_<type_id>` | `cypher/views.rs:27-29` |
| 사람이 읽는 별칭 뷰 | `og_data."<TypeName>"` | `catalog/types.rs:104-106` |
| 프로퍼티 컬럼 | `p_<name>` (소문자화, 비영숫자→`_`) | `catalog/types.rs:53-66` |
| 확장 페이로드 컬럼 | `__ext jsonb` | `catalog/types.rs:414-419` |
| 컴파일러 별칭 | `n1` `adj3` `u4` `vl2` `w1` `e5` `cp7` `lc2` `lp3` `uw1` | `cypher/compile.rs:288-291` + 각 호출부 |
| TypeQL 컴파일러 별칭 | `n1` `at2` `h3` `rp4` `ty5` `p6` `g0` `s1` | `typeql/compile.rs:84-87` |
| TypeQL 투영 컬럼 | `v_<var>` (값), `i_<var>` (식별자) | `typeql/mod.rs:178-184` |
| 내부 타입 이름 | `$` 접두사 (TypeQL 라벨과 충돌 불가) | `typeql/schema.rs:23` `$has` |
| SQL 함수 파라미터 참조 | `<함수명>.<파라미터명>`으로 한정 | `access.sql:19-21` `og_expand.src` |

마지막 항목이 중요하다 — `access.sql` 전체가 `og_expand.src`, `og_vlp.dir` 형태로
파라미터를 한정 참조한다. 컬럼 이름과의 모호성을 없애기 위함이다.

---

## I. 이 문서를 갱신해야 하는 때

- 새 스펙이 추가되어 새 모듈이 생겼을 때
- 위 규칙 중 하나가 의도적으로 깨졌을 때 (그 이유를 여기 기록한다)
- [`11_improvements_code.md`](11_improvements_code.md)의 항목이 해결되어
  '현재 위반' 표시를 지울 수 있을 때

<!-- affects: backend, quality -->
<!-- requires-update: 08_operations/, 99_decisions/ -->
