# TypeQL API — 진입 함수와 문법 경계

> **이 문서가 답하는 질문**
> - TypeQL은 어떤 함수로 들어가고, Cypher와 무엇을 공유하는가?
> - TypeQL 개념이 이 저장소에 어떻게 매핑되는가?
> - 지원되는 스테이지·구문은 정확히 무엇인가?
> - 지원되지 않는 것은 어떻게 보고되는가?
> - `og_typeql`과 `og_typeql_script`의 차이는?

---

## 1. 결정(Decision) — 같은 그래프 위의 두 번째 1급 질의 언어

TypeQL은 별도 저장소를 갖지 않는다. **같은 카탈로그, 같은 저장소, 같은 트랜잭션**
위에서 돈다([engine/src/typeql/mod.rs:1](../../engine/src/typeql/mod.rs#L1)).

- 읽기는 **SQL 문 하나**로 컴파일된다.
- 쓰기는 바인딩 행마다 절차적으로 실행된다.
- 파이프라인은 **감싸기(wrapping)** 로 만들어진다: 형태를 바꾸는 스테이지마다
  이전 결과를 서브쿼리로 감싼다. 그래서 `sort` 다음의 `limit`과 `limit` 다음의
  `sort`가 **실제로 다른 SQL**이 된다(spec 010 FR-036,
  [engine/src/typeql/mod.rs:7](../../engine/src/typeql/mod.rs#L7)).

**방언(dialect)**: TypeDB **3.x** TypeQL. TypeDB 2.x 키워드(`get`, `rule`)는
이름으로 거부하기 위해서만 인식된다([typeql/parser.rs:183](../../engine/src/typeql/parser.rs#L183)).

**트랜잭션**: 스키마/쓰기/읽기 트랜잭션 구분이 없다. PostgreSQL의 트랜잭션이
그 트랜잭션이다([docs/typeql.md:19](../../docs/typeql.md)).

---

## 2. 사실 — 저장 매핑 (spec 010 FR-043)

원문: [docs/typeql.md:26](../../docs/typeql.md). 뷰 정의:
[engine/sql/access.sql:307](../../engine/sql/access.sql#L307), [:324](../../engine/sql/access.sql#L324).

| TypeQL 개념 | 저장 형태 |
|---|---|
| entity type | `og_catalog.type`의 타입, 인스턴스는 `og_data.n_<id>` |
| relation type | 타입 하나, 인스턴스는 **노드로 reify** 되어 `og_data.n_<id>` |
| attribute type | 타입 하나, 인스턴스는 `og_data.a_<id>`에 **값이 UNIQUE** |
| `sub` | `og_catalog.type_parent` + 구간 라벨(spec 002) |
| `relates` / `relates X as Y` | `og_catalog.role` (`parent_role_id`로 특수화) |
| `owns` / `plays` | `og_catalog.og_constraint` |
| 소유(`has`) | 내부 타입 `$has`의 엣지, 인접 세그먼트에 저장 |
| 역할 배정 | `og_data.og_role_player` 행 |

**두 가지 귀결(의도된 것)**

1. **속성은 공유된다, 복사되지 않는다.** `has genre "fiction"`을 가진 두 책은
   *같은* genre 인스턴스를 소유한다. 그래서 `$a has genre $g; $b has genre $g;`가
   문자열 비교가 아니라 순회로 답해진다.
2. **관계는 1급이다.** 세 개 이상의 역할, 속성 소유, 다른 관계의 역할 수행이
   모두 가능하다. 두 끝점짜리 엣지로는 표현할 수 없으므로 관계 인스턴스는 노드다.

매핑을 직접 읽는 방법:

```sql
SELECT * FROM og_typeql_attribute;   -- owner_id, owner_type, attribute_type, value, attribute_id
SELECT * FROM og_typeql_role;        -- relation_id, relation_type, role, player_id, player_type
```

TypeQL 그래프는 Cypher에서도 보인다 — 엔티티 노드, reify된 관계 노드, 속성 노드,
`$has` 엣지로:

```sql
SELECT og_cypher('bookstore', $$ MATCH (b:ebook)-[:`$has`]->(t:title) RETURN t.val $$);
```

---

## 3. 진입 함수

### `og_typeql(graph text, query text, _params jsonb DEFAULT '{}') RETURNS SETOF jsonb`

정의: [engine/src/typeql/mod.rs:48](../../engine/src/typeql/mod.rs#L48) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값(`PARALLEL UNSAFE`)

**무엇을 하는가**: TypeQL 질의를 실행하고 결과 행마다 jsonb 객체 하나를 반환한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | TypeQL 질의. 여러 블록도 허용(내부적으로 `run_script`) |
| `_params` | `jsonb` | 선택 | `'{}'` | ⚠️ **현재 무시된다** — 인자 이름의 밑줄 접두사가 Rust에서 미사용을 뜻한다 |

> ⚠️ `_params`는 시그니처에 존재하지만 함수 본문이 사용하지 않는다
> ([typeql/mod.rs:52](../../engine/src/typeql/mod.rs#L52)). TypeQL에는 파라미터
> 바인딩 경로가 없다 → [12_improvements_api.md](12_improvements_api.md) **API-11**.

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `og_typeql` | `jsonb` | 예 | 읽기: 결과 행. 쓰기: `{"rows": n, "operations": m}` 한 행. `define` 단독: `{"defined": n}` |

**부수 효과**: 성공·실패 모두 `og_data.og_audit`에 `lang = 'typeql'`로 기록
([typeql/mod.rs:115](../../engine/src/typeql/mod.rs#L115)).

**예제**

```sql
SELECT og_typeql('bookstore', $tql$
  match
    $b isa paperback, has title $t;
  select $t;
  sort $t asc;
  limit 5;
$tql$);
```

**실패 조건**: 모든 오류가 `typeql error: <message>` 로 감싸진다
([typeql/mod.rs:60](../../engine/src/typeql/mod.rs#L60)). 상세 메시지는 §5.

---

### `og_typeql_script(graph text, script text) RETURNS int8`

정의: [engine/src/typeql/mod.rs:99](../../engine/src/typeql/mod.rs#L99) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: `.tql` 파일 전체를 순서대로 **한 트랜잭션에서** 실행하고 블록 개수를 반환한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `script` | `text` | 필수 | — | 여러 질의 블록으로 이루어진 스크립트 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int8` | 아니오 | 파싱된 **블록 개수**. 실행된 행 수가 아니다 |

**`og_typeql`과의 차이**

| | `og_typeql` | `og_typeql_script` |
|---|---|---|
| 반환 | `SETOF jsonb` — 마지막 블록의 결과 행 | `int8` — 블록 개수 |
| 감사 로그 | 남긴다 | **남기지 않는다** |
| 오류 위치 | `typeql error: <msg>` | `typeql error in block <i> of <n>: <msg>` ([mod.rs:109](../../engine/src/typeql/mod.rs#L109)) |

**예제** (실제 [tests/typeql/run.py](../../tests/typeql/run.py) 방식)

```sql
SELECT og_typeql_script('bookstore', pg_read_file('/path/to/schema.tql'));
```

**실패 조건**: 파싱 실패 → `typeql parse error: <msg>`.
블록 실행 실패 → 블록 번호를 포함한 메시지. 트랜잭션 전체가 롤백된다.

---

### `og_typeql_sql(graph text, query text) RETURNS text`

정의: [engine/src/typeql/mod.rs:82](../../engine/src/typeql/mod.rs#L82) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: **읽기** 질의가 컴파일된 SQL을 반환한다. Cypher의 `og_cypher_sql`과 같은 투명성(FR-003).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | 읽기 질의 **하나** (`parse`, `parse_script` 아님) |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `text` | 아니오 | 완전한 `SELECT` 문 |

**예제**

```sql
SELECT og_typeql_sql('bookstore', 'match $b isa book, has title $t; select $t;');
```

**실패 조건**
- 파싱 실패 → `typeql parse error: <msg>`
- 쓰기 질의 → `only read queries compile to a single SQL statement`
  ([mod.rs:90](../../engine/src/typeql/mod.rs#L90))
- 컴파일 실패 → `typeql error: <msg>`

**컬럼 이름 규칙 (확인됨)**: 바인딩된 변수 `$v`마다 값 컬럼 `v_<v>`, 식별자가
있으면 id 컬럼 `i_<v>`가 생긴다([typeql/mod.rs:178](../../engine/src/typeql/mod.rs#L178) `vcol`/`icol`).

---

### `og_typeql_check(query text) RETURNS jsonb`

정의: [engine/src/typeql/mod.rs:68](../../engine/src/typeql/mod.rs#L68) · 휘발성: `IMMUTABLE` · 병렬: `PARALLEL SAFE`

**무엇을 하는가**: 파싱만 한다. DB 접근 없음(spec 010 FR-002).

**반환**

| 키 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `ok` | `bool` | 아니오 | 파싱 성공 여부 |
| `queries` | `int` | `ok=false`면 없음 | 스크립트 안의 질의 개수 |
| `write` | `bool` | `ok=false`면 없음 | `insert`/`put`/`delete`/`update`/`define`/`undefine` 중 하나라도 있으면 `true` |
| `error` | `text` | `ok=true`면 없음 | 파서 메시지 |

**예제**

```sql
SELECT og_typeql_check('match $b isa book; select $b;');
-- {"ok": true, "queries": 1, "write": false}
```

**실패 조건**: 없음 — `ok: false`로 보고한다.

---

### `og_typeql_schema(graph text) RETURNS text`

정의: [engine/src/typeql/dump.rs:10](../../engine/src/typeql/dump.rs#L10) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 그래프의 스키마를 TypeQL `define` 블록으로 렌더링한다(spec 010 T047 / FR-013).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `text` | 아니오 | `define` 으로 시작하는 TypeQL 텍스트 |

내부 타입(`$has` 등, `name LIKE '$%'`)은 제외된다
([typeql/dump.rs:20](../../engine/src/typeql/dump.rs#L20)).
출력 순서는 `entity` → `relation` → `attribute`, 각 종류 안에서 이름순.

**왕복 검증**이 이 함수의 존재 이유다 — `define`이 정보를 버리면 덤프가 입력을
재현하지 못하고 SC-006이 실패한다([typeql/dump.rs:3](../../engine/src/typeql/dump.rs#L3)).

**예제**

```sql
SELECT og_typeql_schema('bookstore');
-- define
-- entity book @abstract, owns isbn-13, plays contribution:work;
-- ...
```

**실패 조건**: 그래프 없음 → `graph '<g>' does not exist`.

---

## 4. 지원되는 TypeQL — 사실

원문 지원 표: [docs/typeql.md:74](../../docs/typeql.md).
아래는 AST/파서/컴파일러에서 재확인한 것이다.

### 4.1 스테이지 (`Stage`)

정의: [engine/src/typeql/ast.rs:16](../../engine/src/typeql/ast.rs#L16)

| 스테이지 | 지원 상태 |
|---|---|
| `define` | 지원 |
| `undefine` | **파싱만** — 실행 시 거부 (§5.1) |
| `match` | 지원 |
| `insert` | 지원 |
| `put` | 지원 (매치 후 없으면 삽입) |
| `update` | 지원 — **`has` 교체만** ([write.rs:503](../../engine/src/typeql/write.rs#L503)) |
| `delete` | 지원 |
| `select` | 지원 |
| `sort` | 지원 |
| `limit` / `offset` | 지원 |
| `distinct` | 지원 |
| `reduce` | 지원 |
| `fetch` | 지원 |

### 4.2 `define` 문법

- 종류: `entity`, `relation`, `attribute`
- 구조: `sub`, `owns`, `relates`, `relates X as Y`, `plays R:r`
- 값 타입: `string`, `integer`, `double`, `decimal`, `boolean`, `date`,
  `datetime`, `datetime-tz`, `duration`
- 어노테이션: `@abstract`, `@key`, `@unique`, `@card(n..m)`, `@values(...)`,
  `@range(a..b)`
- **수용되지만 무시되는 어노테이션**: `@distinct`, `@cascade`, `@independent`,
  `@subkey` — 이 엔진이 구체화하는 것을 아무것도 제약하지 않기 때문
  ([docs/typeql.md:110](../../docs/typeql.md))
- **전방 참조 허용**: `plays contribution:work`가 `relation contribution` 선언보다
  먼저 나와도 된다. `define`은 여러 패스로 돈다.
- **멱등**: 같은 `define`을 다시 돌리면 무연산.

**쓰기 시점에 강제되는 것**: `@key`, `@unique`, `@card`의 **상한**, `@values`,
`@range`, 그리고 `@abstract` 타입의 인스턴스화 거부
([engine/src/typeql/write.rs:278](../../engine/src/typeql/write.rs#L278) 부근).

### 4.3 읽기 파이프라인

```typeql
match
  $b isa paperback, has title $t, has price $p;
  $p > 10;
select $t, $p;
sort $p desc;
limit 10;
```

- 변수를 공유하지 않는 패턴 그룹은 각각 컴파일된 뒤 `CROSS JOIN` 된다
  ([typeql/mod.rs:199](../../engine/src/typeql/mod.rs#L199) `compile::components`).
- 각 형태 변경 스테이지는 이전 SQL을 `SELECT … FROM (…) s<n>` 로 감싼다.

---

## 5. 지원되지 않는 것 — **어떻게 보고되는가**

Cypher의 `UNION`과 달리 TypeQL 쪽은 **전부 명시적으로 거부**된다. 조용히 버려지는
구문은 확인되지 않았다.

### 5.1 실행 시 거부

| 구문 | 오류 메시지 | 위치 |
|---|---|---|
| `undefine` | `'undefine' is not implemented yet (spec 010 phase 5)` | [typeql/mod.rs:152](../../engine/src/typeql/mod.rs#L152) |
| 쓰기 스테이지 뒤의 형태 변경 스테이지 | `'<stage>' after a write stage is not supported yet: run it as a separate query` | [typeql/mod.rs:501](../../engine/src/typeql/mod.rs#L501) |
| `update`에 `has` 아닌 것 | `'update' only supports 'has' replacements` | [typeql/write.rs:503](../../engine/src/typeql/write.rs#L503) |

### 5.2 파싱 시 거부

| 구문 | 오류 메시지 | 위치 |
|---|---|---|
| `with fun …` (질의 지역 함수) | `<pos>: query-local functions ('with fun') are not supported yet (spec 010 phase 6). declare the function in the schema instead` | [parser.rs:178](../../engine/src/typeql/parser.rs#L178) |
| `get` / `rule` (TypeDB 2.x) | `<pos>: '<kw>' is TypeDB 2.x syntax. this engine implements TypeQL 3.x — use 'select'/'fetch' instead of 'get'` | [parser.rs:184](../../engine/src/typeql/parser.rs#L184) |
| 알 수 없는 스테이지 | `<pos>: expected a query stage (define, match, insert, put, delete, update, select, sort, limit, offset, distinct, reduce, fetch), found '<tok>'` | [parser.rs:192](../../engine/src/typeql/parser.rs#L192) |
| 빈 질의 | `empty query` | [parser.rs:43](../../engine/src/typeql/parser.rs#L43) |

### 5.3 ⚠️ 스키마 UDF — 파싱은 되지만 **평가되지 않는다**

`define` 안의 함수 선언(`fun name(...) -> T: … return …;`)은 파서가 본문을
**건너뛴다** — `return`을 브레이스 깊이 0에서 찾아 그 `;`까지 스킵한다
([engine/src/typeql/parser.rs:919](../../engine/src/typeql/parser.rs#L919)).

즉 UDF는 **파싱과 왕복(round-trip)만** 되고 평가 경로가 없다.
README 스펙 상태표의 "spec 010 — partial (UDF는 파싱/왕복만)"이 이것이다.

> ⚠️ 함수를 선언한 뒤 질의에서 호출하면, 그 호출은 UDF로 인식되지 않고
> 일반 표현식 파서로 넘어가
> `<pos>: unexpected name '<name>' in an expression`
> ([parser.rs:713](../../engine/src/typeql/parser.rs#L713))가 된다.
> "선언은 성공했는데 사용은 알 수 없는 이름"이라는 혼란스러운 조합
> → [12_improvements_api.md](12_improvements_api.md) **API-12**.

### 5.4 컴파일 시 거부 (주요 항목)

| 상황 | 오류 메시지 | 위치 |
|---|---|---|
| `match` 스테이지 없음 | `a read query needs a 'match' stage` | [typeql/mod.rs:194](../../engine/src/typeql/mod.rs#L194) |
| 아무 변수도 바인딩 안 됨 | `the 'match' stage binds no variables` | [typeql/mod.rs:241](../../engine/src/typeql/mod.rs#L241) |
| `select $v`가 미바인딩 | `'select $<v>': that variable is not bound` | [typeql/mod.rs:257](../../engine/src/typeql/mod.rs#L257) |
| `sort $v`가 미바인딩 | `'sort $<v>': that variable is not bound` | [typeql/mod.rs:281](../../engine/src/typeql/mod.rs#L281) |
| 속성 타입 없는 `has $a` | `'has $a' without an attribute type is not supported: name the attribute type` | [compile.rs:319](../../engine/src/typeql/compile.rs#L319) |
| 속성 타입이 아님 | `'<name>' is not an attribute type` | [compile.rs:325](../../engine/src/typeql/compile.rs#L325) |
| 타입 변수를 인스턴스로 사용 | `'$<v>' is a type variable, not an instance` | [compile.rs:303](../../engine/src/typeql/compile.rs#L303) |
| 역할 없는 관계 변수 | `relation variable '$<rel>' has no roles to match` | [compile.rs:413](../../engine/src/typeql/compile.rs#L413) |
| 미바인딩 변수 참조 | `'$<v>' is not bound by any pattern` | [compile.rs:459](../../engine/src/typeql/compile.rs#L459) |
| 인스턴스화 불가 속성 타입 | `this attribute type has no instantiable subtype` | [compile.rs:147](../../engine/src/typeql/compile.rs#L147) |

### 5.5 쓰기 시 거부 (주요 항목)

| 상황 | 오류 메시지 | 위치 |
|---|---|---|
| `insert`의 미바인딩 변수 | `insert refers to unbound variable '$<v>'` | [write.rs:84](../../engine/src/typeql/write.rs#L84) |
| 값 없는 `has` | `'has <a>' in an insert needs a value` | [write.rs:89](../../engine/src/typeql/write.rs#L89) |
| 속성 타입 없는 `has` | `'has' in an insert needs an attribute type` | [write.rs:91](../../engine/src/typeql/write.rs#L91) |
| `@range` 하한 위반 | `<value> is below the declared range of '<attr>' (<lo>..)` | [write.rs:288](../../engine/src/typeql/write.rs#L288) |
| `@range` 상한 위반 | `<value> is above the declared range of '<attr>' (..<hi>)` | [write.rs:293](../../engine/src/typeql/write.rs#L293) |
| `delete`의 미바인딩 변수 | `delete refers to unbound variable '$<attr>'` | [write.rs:472](../../engine/src/typeql/write.rs#L472) |
| 쓰기 스테이지의 미지원 표현식 | `expressions of this shape are not available in a write stage` | [write.rs:642](../../engine/src/typeql/write.rs#L642) |
| 미지원 저장 타입 | `unsupported storage type '<t>' for '<attr>'` | [write.rs:672](../../engine/src/typeql/write.rs#L672) |

---

## 6. 금지 / 필수

- **금지**: `og_typeql(..., params)`에 값을 넘기고 바인딩될 것으로 기대하지 말 것.
  **무시된다**(§3, API-11). 값은 질의 텍스트에 넣어야 하며, 따라서
  **신뢰할 수 없는 입력으로 TypeQL 질의를 조립하지 말 것.**
- **금지**: `undefine`을 사용하지 말 것 — 아직 구현되지 않았다.
- **금지**: 쓰기 스테이지 뒤에 `select`/`sort`/`limit`을 붙이지 말 것.
  별도 질의로 나눌 것.
- **필수**: 여러 블록짜리 스크립트에는 `og_typeql_script`를 쓸 것 —
  오류 메시지에 블록 번호가 붙는다.
- **필수**: 스키마 왕복을 검증하려면 `og_typeql_schema` → 새 그래프에
  `og_typeql_script` → 결과 비교 순서를 쓸 것(테스트 하네스가 하는 방식,
  [tests/typeql/run.py](../../tests/typeql/run.py)).

---

## 7. 관련 문서

- 원문 지원 표와 저장 매핑 → [docs/typeql.md](../../docs/typeql.md)
- 같은 그래프의 Cypher 표면 → [03_cypher.md](03_cypher.md)
- 타입/역할 DDL(SQL 경로) → [01_graph_ddl.md](01_graph_ddl.md)
- 오류 체계 → [11_errors.md](11_errors.md)

<!-- affects: api, backend -->
<!-- requires-update: 02_api/11_errors.md, 02_api/12_improvements_api.md -->
