# TypeQL 엔진 — define / insert / put / match / fetch / delete / update

> **이 문서가 답하는 질문**
> - TypeQL 질의는 왜 "절의 트리"가 아니라 "스테이지 파이프라인"인가?
> - `define`은 왜 5번 순회하는가?
> - 엔티티/관계/속성이 각각 어떤 물리 구조에 저장되는가?
> - `match`가 SQL이 되는 세 가지 조인은 무엇인가?
> - 속성 값 중복 제거(interning)는 어디서 강제되는가?
> - Cypher 표면과 어디서 만나는가?

원문 근거: [`docs/typeql.md`](../typeql.md), spec 010.

---

## 1. 결정 — 파이프라인 AST

`engine/src/typeql/ast.rs:1-6`:

> A TypeQL query is a **pipeline of stages**, not a tree of clauses. Each stage
> consumes the row stream the previous one produced. Keeping that shape in the AST
> is what makes `sort` before `limit` mean something different from `limit`
> before `sort` (FR-036) without any special-casing later.

```rust
pub struct Query { pub stages: Vec<Stage> }

pub enum Stage {
    Define(Vec<Definition>), Undefine(Vec<Definition>),
    Match(Vec<Pat>), Insert(Vec<Pat>), Put(Vec<Pat>), Update(Vec<Pat>), Delete(Vec<DeleteItem>),
    Select(Vec<String>), Sort(Vec<(String, bool)>), Limit(i64), Offset(i64), Distinct,
    Reduce { assigns: Vec<Reduction>, groupby: Vec<String> },
    Fetch(FetchDoc),
}
```

구현에서 이 형태가 그대로 드러난다 — `compile_read`(`typeql/mod.rs:247-368`)는
스테이지마다 이전 SQL을 **서브쿼리로 감싼다**:

```rust
p.sql = format!("SELECT * FROM ({}) s{depth} ORDER BY {}", p.sql, order.join(", "));
```

`sort` → `limit`이면 `SELECT * FROM (SELECT * FROM (...) s1 ORDER BY ...) s2 LIMIT n`,
`limit` → `sort`면 반대 중첩. 정규화하지 않는다.

---

## 2. 사실 — 렉서의 두 가지 특이점

`engine/src/typeql/lexer.rs:1-10`:

1. **라벨에 하이픈이 들어간다.** `isbn-13`, `order-line`, `start-timestamp`는
   뺄셈이 아니라 하나의 이름이다. `-`는 식별자 문자 사이에 정확히 놓였을 때만 흡수되므로
   `1 - $d`와 `$a-$b`는 여전히 산술이다.
2. **datetime이 따옴표 없이 온다.** `2023-12-02T00:00:00`이 리터럴이며,
   첫 글자가 숫자다 → 숫자로 시작한다고 해서 수치 리터럴이 아니다.

토큰 종류 (`lexer.rs:15-27`): `Ident` / `Var($x)` / `Str` / `Int` / `Float` / `DateTime` / `Sym` / `Eof`.
**키워드 토큰이 없다** — 모든 키워드는 `Ident`이고 파서가 위치로 판단한다.
그래서 `match`라는 이름의 속성을 쓸 수 있다.

기호 목록 (`lexer.rs:52-55`): `.. == != <= >= -> = < > + - * / % ; , ( ) { } [ ] : . @ ? |`.

---

## 3. 사실 — `define` (5 패스)

`engine/src/typeql/schema.rs:56-128` `run_define`.

패스를 나눈 이유 (`schema.rs:4-9`, `schema.rs:78-82`): 실제 `.tql` 파일은 전방 참조를 마음껏 한다.
`entity book … owns isbn`이 `attribute isbn, value string`보다 훨씬 앞에 온다.

| 패스 | 라인 | 하는 일 |
|---|---|---|
| 0 | 57 | `ensure_has_type(gid)` — 내부 `$has` 관계 타입 확보 |
| 1 | 59–64 | **껍데기 선언** — `declare_shell()`: 이름/종류/추상 여부만 `og_catalog.type`에 |
| 2 | 66–75 | **상속 연결** — `link_sub()` → 변화가 있으면 `relabel_graph(gid)` **1회** |
| 3 | 77–95 | **스토리지 생성** + 일반 롤 선언 |
| 4 | 97–105 | **롤 특수화** (`relates author as contributor`) |
| 5 | 107–124 | `owns` / `plays` / 값 제약 / 함수 저장 |
| 끝 | 126 | `bump_schema_version(gid, "typeql define")` |

모든 패스가 **멱등**이다 (`schema.rs:11-12`, FR-012). 같은 스키마를 두 번 로드해도 무해하다.
`declare_shell`(`schema.rs:134-165`)은 이미 있으면 종류만 확인하고 반환하며,
종류가 다르면 명시적으로 거절한다.

### 3.1 물리 구조 매핑

| TypeQL | 종류 | 스토리지 | 근거 |
|---|---|---|---|
| `entity book` | `'e'` | `og_data.n_<tid> (id int8 PK, __ext jsonb)` | `schema.rs:223-236` |
| `relation authoring` | `'r'` | 같은 **노드** 테이블 — 관계가 **reify**된다 | `schema.rs:223-236` |
| `attribute title, value string` | `'a'` | `og_data.a_<tid> (id int8 PK, val text NOT NULL UNIQUE, __ext jsonb)` | `schema.rs:270-278` |
| 속성 소유 | — | 내부 관계 타입 `$has`의 엣지 | `schema.rs:23`, `write.rs:381-407` |
| 롤 배정 | — | `og_data.og_role_player (edge_id, role_id, player_id)` | `write.rs:446-451` |

**`val` 컬럼의 `UNIQUE`가 기능적으로 필수다** (`schema.rs:271-272`):

> UNIQUE on the value is load-bearing, not defensive: it is what makes two owners
> of "fiction" share one instance (FR-016).

`$has`라는 이름은 `$` 접두사 때문에 TypeQL 라벨과 절대 충돌하지 않는다 (`schema.rs:21-22`).
즉 언어에는 보이지 않으면서 카탈로그에서는 평범한 시민이다.

속성 타입에는 `val`이 프로퍼티로도 등록된다 (`schema.rs:281-288`) —
그래야 Cypher 표면에서 `t.val`로 읽힌다.

### 3.2 값 타입 매핑

`schema.rs:28-46` `value_type_sql`:

| TypeQL | PostgreSQL |
|---|---|
| `string` | `text` |
| `integer` / `long` | `int8` |
| `double` | `float8` |
| `decimal` | `numeric` |
| `boolean` | `bool` |
| `date` | `date` |
| `datetime` | `timestamp` |
| `datetime-tz` | `timestamptz` |
| `duration` | `interval` |

값 타입은 `og_catalog.og_constraint`에 `kind = 'value'` 행으로 저장된다 (`schema.rs:312-314`).
카탈로그가 단일 진실 원천이라야 `og_typeql_schema()` 덤프가 입력을 재현할 수 있다.

`owns` / `plays` / `values` / `range` 어노테이션도 같은 테이블에 들어간다
(`schema.rs:424,437,442`, JSON 직렬화는 `annotations_json` `schema.rs:445-466`).

---

## 4. 사실 — `match` → SQL

`engine/src/typeql/compile.rs`. 모듈 주석(`compile.rs:4-12`)이 세 개의 조인을 명시한다.

### 4.1 `isa` — 구간 라벨로 확장된 타입 집합

`pat_isa`(`compile.rs:261-294`):

```sql
CROSS JOIN og_data.og_node n1
-- WHERE n1.type_id = ANY(ARRAY[3,7,9]::int4[])
```

`ARRAY[...]`는 `labeling::og_subtypes(tid)`의 결과다 (`compile.rs:131-134`). 재귀 없음.

속성 타입(`kind == 'a'`)이면 노드 레지스트리가 아니라 **구체 속성 테이블들의 UNION ALL**을 스캔한다
(`concrete_attr_tables` `compile.rs:136-150`).

이미 바인딩된 변수에 `isa`가 또 붙으면 조인을 늘리지 않고 **식별자에서 타입을 뽑아 비교**한다
(`compile.rs:267-280`).

### 4.2 `has` — 인접 세그먼트 + 식별자 시프트

`pat_has`(`compile.rs:315-380`). 생성되는 SQL:

```sql
CROSS JOIN LATERAL (
  SELECT h5v.id AS id, h5v.val AS val
    FROM og_data.og_adj h5a
    CROSS JOIN LATERAL unnest(h5a.nbr) AS h5u(nbr)
    JOIN (SELECT id, val FROM og_data.a_12) h5v ON h5v.id = h5u.nbr
   WHERE h5a.src = n1.id
     AND h5a.dir = 'o'::"char"
     AND h5a.etype = 8
     AND (((h5u.nbr) >> 36) & 262143)::int4 = ANY(ARRAY[12]::int4[])
     AND (h5v.val) = ('fiction')
) h5
```

주목할 점:

- **속성 타입 필터가 조인이 아니라 시프트-마스크다** (`type_of_id` `compile.rs:161-163`).
  spec 001이 타입을 식별자 안에 넣었기 때문에 카탈로그 조회가 필요 없다.
- **값 술어가 같은 LATERAL 안에 있다** (`compile.rs:328-332`). 이게 없으면
  독립 변수가 여러 개인 `match`가 값 필터를 적용하기 전에 교차곱을 만든다.
- `has_tid`는 컴파일러 생성 시점에 한 번 조회된다 (`compile.rs:61`).

### 4.3 `role` — `og_role_player` + 롤 특수화 확장

`pat_role`(`compile.rs:382-439`):

```sql
CROSS JOIN og_data.og_role_player rp7
-- WHERE rp7.edge_id = n3.id AND rp7.player_id = n1.id
--   AND rp7.role_id = ANY(ARRAY[4,5]::int4[])
```

롤 id 집합은 두 가지로 만들어진다:

- 롤 이름이 있으면 `role_with_specialisations(base)` (`compile.rs:641-658`) —
  `og_catalog.role.parent_role_id`를 따라 내려가는 재귀 CTE.
  (헌법이 금지하는 건 **타입 계층** 재귀이지 롤 특수화가 아니다.)
- 없으면(위치 인자) `all_roles_of(rel_tid)` (`compile.rs:661-677`).

**위치 인자 중복 방지** (`compile.rs:422-436`): `($a, $b) isa rating`이 같은 플레이어를
자기 자신과 매칭하지 않도록, 같은 관계 변수의 이전 `rp` 별칭들과
`NOT (role_id = ... AND player_id = ...)`를 건다.

### 4.4 부정과 분리

- `not { ... }` → `NOT EXISTS (<서브쿼리>)` (`compile.rs:220-224`)
- `{ ... } or { ... }` → `(EXISTS (...) OR EXISTS (...))` (`compile.rs:225-232`)

서브쿼리는 `Compiler::nested(self)`(`compile.rs:73-82`)로 만든다.
부모의 바인딩을 복사하되 `external`에 넣어 **다시 물질화하지 않고 상관 참조**하게 한다.
별칭 카운터도 `parent.n + 1000`에서 시작해 충돌을 피한다.

### 4.5 ★ 연결 성분 분해 — 이 파일의 가장 큰 성능 결정

`compile::components(&pats)` (`compile.rs:735-774`). 주석(`compile.rs:728-734`):

> A `match` listing sixteen independently-pinned cities is one conjunction of
> sixteen unrelated patterns. Handed to the planner as one flat join it has 16!
> orderings, GEQO picks badly, and the query builds a cross product before any
> name filter applies. Compiling each group as its own subquery removes the choice
> entirely — there is nothing to get wrong between groups, and `from_collapse_limit`
> keeps PostgreSQL from flattening them back together.

Union-Find(`compile.rs:737-744`)로 변수를 공유하는 패턴끼리 묶고,
그룹마다 별도 `Compiler`로 컴파일한 뒤 `CROSS JOIN`으로 잇는다 (`typeql/mod.rs:206-243`).

단위 테스트 2개가 이를 고정한다 (`compile.rs:791-816`):
`independent_variables_split_into_groups`, `shared_variables_stay_together`.

### 4.6 변수 투영 규약

`typeql/mod.rs:167-184` — 바인딩된 변수 `$x`는 **두 컬럼**이 된다:

- `v_x` — 값 (`var_value`, `compile.rs:583-592`)
- `i_x` — 식별자 (`var_id`, `compile.rs:595-605`) — 있는 경우에만

`Bind` 5종 (`compile.rs:23-37`): `Instance` / `Attr` / `Value` / `Type` / `Column`.
`Column`은 바깥 파이프라인 스테이지가 이미 투영한 컬럼을 가리키며,
이게 있어야 `fetch`가 `sort`/`limit`에 감싸인 뒤에도 살아남는다.

---

## 5. 사실 — 읽기 스테이지 실행

`typeql/mod.rs:247-368` `compile_read`.

| 스테이지 | 생성 SQL | 라인 |
|---|---|---|
| `select $a, $b` | `SELECT v_a, i_a, v_b FROM (...) sN` | 254–272 |
| `distinct` | `SELECT DISTINCT * FROM (...) sN` | 273–276 |
| `sort $a asc` | `SELECT * FROM (...) sN ORDER BY v_a ASC` | 277–288 |
| `limit n` | `SELECT * FROM (...) sN LIMIT n` | 289–292 |
| `offset n` | `SELECT * FROM (...) sN OFFSET n` | 293–296 |
| `reduce $c = count; groupby $a` | `SELECT v_a, i_a, count(*) AS v_c FROM (...) sN GROUP BY v_a, i_a` | 297–331 |
| `fetch { ... }` | `SELECT jsonb_build_object(...) AS row FROM (...) sN` | 332–349 |
| (fetch 없음) | 변수 전부를 jsonb 객체로 | 356–367 |

`reduce`에서 식별자 컬럼도 `GROUP BY`에 들어간다 (`typeql/mod.rs:315-324`).
함수 종속이지만 그걸 아는 건 카탈로그뿐 플래너가 아니다.

집계 매핑 (`agg_sql` `typeql/mod.rs:370-392`):
`count sum max min mean(→avg) median(→percentile_cont) std(→stddev) list(→jsonb_agg)`.

### 5.1 `fetch`

`fetch_object`(`typeql/mod.rs:394-409`)가 `jsonb_build_object`를 만든다. 항목 4종:

| 항목 | 컴파일 |
|---|---|
| `"k": $x.attr` | `to_jsonb(attr_scalar(...))` — 상관 스칼라 서브쿼리 `LIMIT 1` (`compile.rs:497-513`) |
| `"k": [$x.attr]` | `attr_list(...)` — `COALESCE((SELECT jsonb_agg(...)), '[]'::jsonb)` (`compile.rs:516-532`) |
| `"k": { ... }` | 중첩 객체 |
| `"k": [ match … fetch … ]` | `sub_fetch(...)` (`typeql/mod.rs:413-432`) |

**서브페치는 바깥 행을 지울 수 없다** (`typeql/mod.rs:411-412`):
상관 집계이므로 매치가 없으면 그냥 빈 배열이 된다 (FR-035).

```sql
COALESCE((SELECT jsonb_agg(<obj>) FROM ... WHERE ...), '[]'::jsonb)
```

---

## 6. 사실 — 쓰기 스테이지

`typeql/mod.rs:447-510` `run_write_pipeline`.

```
선행 match가 있으면 → compile_match로 SQL 만들고 실행 → Env 목록 생성
없으면 → 빈 Env 하나
Env 행마다:
    Insert → write::run_insert
    Put    → write::run_put
    Update → write::run_update
    Delete → write::run_delete
반환: [{"rows": N, "operations": M}]
```

`Env`(`write.rs:20-49`)는 `HashMap<String, EnvVal>`이고, `EnvVal`은
`Instance(i64)` / `Attr(i64, Value)` / `Value(Value)` 셋 중 하나다.

### 6.1 `insert` — 4단계 순서가 의미를 갖는다

`write.rs:55-109`:

| 단계 | 라인 | 이유 |
|---|---|---|
| 1. 인스턴스 생성 (`isa`) | 57–65 | 롤 플레이어와 소유자가 먼저 존재해야 한다 |
| 2. 롤 배정 (`links`) | 67–74 | |
| 3. 소유 (`has`) | 76–100 | |
| 4. `let` 계산 | 102–107 | |

`create_instance`(`write.rs:179-205`): `og_node` 레지스트리 + 타입 테이블 삽입.
속성 타입에 `isa`를 쓰면 명시적으로 거절한다 (`write.rs:188-193`) —
속성은 값을 소유해서 만들지 `isa`로 만들지 않는다.

### 6.2 속성 interning

`intern_attribute`(`write.rs:213-262`):

```
1. 속성 타입 확인 (kind == 'a')
2. 소유자가 이 속성을 owns 하는지 검사 (schema::owned_attribute)   ← 없으면 오류
3. 값 타입 → SQL 리터럴 (typed_literal)
4. 값 제약 검사 (@values / @range)                                write.rs:264-298
5. 소유 제약 검사 (@key / @unique / @card)                        write.rs:300-361
6. SELECT id FROM <table> WHERE val = <lit>   → 있으면 그 id 재사용
7. 없으면 alloc_id + og_node + <table> 삽입
```

`@key` / `@unique`는 **소유자 타입 계통 전체**에서 검사한다 (`write.rs:314-336`).
`root_of(owner_tid)`(`write.rs:365-377`)가 최상위 supertype을 찾고, 그 서브타입 전체가 범위다 —
TypeDB의 스코프와 맞추기 위함이다.

`link_has`(`write.rs:381-407`)는 spec 001 FR-012를 그대로 따른다:
레지스트리(`og_edge`) + 타입 테이블 + **양방향 인접 세그먼트**.

### 6.3 `put`

`write.rs:112-120` — `find_one`으로 먼저 찾고, 없으면 `run_insert`.

`find_one`(`write.rs:124-173`)은 현재 `Env`의 값을 `seed_column`으로 고정점으로 박은 뒤
패턴을 컴파일해 `LIMIT 1` SELECT를 만든다.

> **주의**: `put`은 원자적이지 않다. SELECT → INSERT 사이에 락이 없으므로
> 동시 실행 시 둘 다 삽입할 수 있다. 속성 값의 `UNIQUE`가 마지막 방어선이지만
> 인스턴스에는 그런 게 없다. → [`07_transactions_and_concurrency.md`](07_transactions_and_concurrency.md).

### 6.4 `update`

`write.rs:500-531` — `has`만 지원한다. 해당 속성 타입의 **현재 값을 전부 끊고** `run_insert`.

### 6.5 `delete`

`write.rs:459-497`:

| 형태 | 동작 |
|---|---|
| `delete $x;` | `delete_instance(gid, id)` |
| `delete has $a of $x;` | `unlink_has(gid, owner, attr)` |
| `delete links (role: $p) of $r;` | `og_role_player`에서 해당 행 삭제 |

`delete_instance`(`write.rs:552-604`)는 **연쇄적**이다 (`write.rs:550-551`, FR-038):

1. 이 인스턴스가 걸린 `$has` 엣지를 양방향으로 모두 끊는다.
2. 이 인스턴스가 롤을 맡은 **관계 인스턴스를 재귀적으로 삭제한다** —
   플레이어가 빠진 관계는 작아진 관계가 아니라 깨진 관계이므로.
3. `og_role_player`, 타입 테이블, `og_node`, `og_adj`에서 제거.

> **주의**: 2번의 재귀에는 깊이 제한이나 방문 집합이 없다 (`write.rs:593-595`).
> 관계가 관계의 플레이어인 순환 구조에서 무한 재귀 가능성이 있다. → `CODE-17`.

---

## 7. 사실 — 미구현 / 제한

| 항목 | 상태 | 근거 |
|---|---|---|
| `undefine` | 미구현, 명시적 오류 | `typeql/mod.rs:151-153` |
| `redefine` | 렉서 키워드 목록에만 존재 | `typeql/parser.rs:282` |
| 사용자 정의 함수 (`fun`) | 파싱·저장·덤프만. 평가 없음 | `typeql/schema.rs:505-519`, `compile.rs:484-490` |
| `has $a` (속성 타입 생략) | 거절 | `compile.rs:317-322` |
| 쓰기 스테이지 뒤의 읽기 스테이지 | 거절 | `typeql/mod.rs:499-505` |
| `og_typeql`의 `_params` | **무시된다** | `typeql/mod.rs:52` → `CODE-18` |
| 표현식 함수 | `round abs floor ceil length`만 | `compile.rs:476-491` |

---

## 8. 사실 — Cypher 표면과의 접점

**같은 그래프다.** 접점은 세 곳이다.

1. `cypher/views.rs:55-78` `concrete_tables` — 노드로 취급할 스토리지 테이블을
   `og_data.n\_%` **또는 `og_data.a\_%`** 패턴으로 고른다. 즉 TypeQL 속성 인스턴스는
   Cypher에게 노드다 (`views.rs:60-64`, spec 010 FR-040).

2. `engine/sql/access.sql:307-338` — 매핑을 SQL로 명시한 뷰 2개:
   - `og_typeql_attribute` — (소유자, 속성) 쌍 하나당 1행
   - `og_typeql_role` — reify된 관계의 롤 배정 하나당 1행

3. README가 보여주는 교차 질의:
   ```sql
   SELECT og_cypher('bookstore', $$ MATCH (b:ebook)-[:`$has`]->(t:title) RETURN t.val $$);
   ```
   `$has`를 백틱으로 감싸면 Cypher에서 그대로 관계 타입으로 쓸 수 있다.

---

## 9. 결정 요약

| 결정 | 근거 | 대가 |
|---|---|---|
| 스테이지를 서브쿼리로 감싼다 | `ast.rs:1-6` (FR-036) | 스테이지가 많으면 중첩이 깊어짐 |
| 연결 성분을 분해해 그룹별 서브쿼리 | `compile.rs:728-734` | `from_collapse_limit`에 의존 |
| 관계를 노드로 reify | `schema.rs:223-236` | 관계에 롤이 3개 이상 가능 / Cypher에서 노드로 보임 |
| 속성을 값으로 중복 제거 | `schema.rs:271-272` (FR-016) | `val`에 UNIQUE → 대용량 텍스트 속성에 인덱스 부담 |
| 속성 타입 필터를 식별자 시프트로 | `compile.rs:159-163` | 없음 |
| 쓰기는 행마다 Rust 절차 실행 | `typeql/mod.rs:3-5` | 배치 쓰기가 느림 |

---

## 금지 / 필수

- **금지**: `define`의 패스 순서를 바꾸는 것. 전방 참조가 깨진다 (`schema.rs:78-82`).
- **금지**: `og_data.a_<tid>.val`의 `UNIQUE`를 제거하는 것. FR-016이 그 인덱스에 의존한다.
- **금지**: `$has` 타입 이름을 바꾸거나 `$` 접두사를 떼는 것.
  TypeQL 라벨과 충돌 불가능성이 그 접두사에서 온다 (`schema.rs:21-22`).
- **금지**: TypeQL 쓰기 경로에서 새로 SQL 문자열에 값을 끼워 넣는 것.
  기존 `typed_literal`(`write.rs:649-674`)도 개선 대상이다 (`CODE-08`).
- **필수**: 새 스테이지를 추가하면 `stage_name()`(`typeql/mod.rs:512-529`)과
  `compile_read`의 match 양쪽을 갱신한다.
- **필수**: 새 패턴 종류를 추가하면 `vars_of()`(`compile.rs:684-725`)도 갱신한다.
  빠뜨리면 연결 성분 분해가 관련 패턴을 다른 그룹으로 보내 답이 틀린다.

<!-- affects: backend, data, api -->
<!-- requires-update: 02_api/03_typeql_api.md, 06_data/ -->
