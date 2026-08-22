# Cypher 프런트엔드 — 렉서 → 파서 → AST

> **이 문서가 답하는 질문**
> - 어떤 토큰이 인식되고 어떤 것이 인식되지 않는가?
> - 예약어는 무엇이고, 왜 `INDEX` / `FOR` / `REQUIRE`는 예약어가 아닌가?
> - 파서가 지원하는 문법 커버리지는 어디까지인가?
> - 지원하지 않는 문법은 어떻게 실패하는가?
> - AST에서 컴파일러가 실제로 읽는 정보는 무엇인가?

---

## 1. 사실 — 파이프라인

```
&str  ──Lexer::tokenize()──▶  Vec<Token>  ──Parser::parse_query()──▶  Query
      cypher/lexer.rs:81                    cypher/parser.rs:124
```

진입점은 `cypher::parser::parse(src)` 하나다 (`parser.rs:22-28`).
어휘 오류든 문법 오류든 `Err(String)`으로 나온다. `panic!`은 없다.

---

## 2. 사실 — 렉서 (`cypher/lexer.rs`, 302줄)

### 2.1 토큰 종류

`lexer.rs:3-15`:

| 변형 | 예 | 비고 |
|---|---|---|
| `Ident(String)` | `Person`, `name` | 원문 대소문자 보존 |
| `QuotedIdent(String)` | `` `my label` `` | 백틱 안은 무엇이든 |
| `Keyword(String)` | `match` | **소문자로 정규화** 후 저장 |
| `Int(i64)` | `42` | |
| `Float(f64)` | `3.5`, `1e3` | |
| `Str(String)` | `'x'`, `"x"` | 이스케이프 `\n \t \r \0` 처리 |
| `Param(String)` | `$name` | `$` 뒤가 비면 오류 |
| `Punct(String)` | `->`, `<->`, `..` | |
| `Eof` | | |

`Token`은 `tok` 외에 `pos`(바이트 오프셋)와 **`raw`(원문 철자)**를 함께 들고 다닌다
(`lexer.rs:34-43`). `raw`가 필요한 이유:

> 키워드는 소문자로 정규화해 대소문자 무시 매칭을 하지만, 키워드가 **이름 자리**에 올 수 있다 —
> `[r:CONTAINS]`, `(n:Order)`. 거기서는 원문 철자가 타입 이름이다.

`Parser::name()`(`parser.rs:108-118`)이 `Tok::Keyword`를 만나면 `raw`를 반환하는 이유가 이것이다.

### 2.2 예약어 (`lexer.rs:17-23`)

```
match optional where return with create merge set remove delete
detach order by skip limit distinct as and or not xor in
is null true false unwind union all asc desc starts ends
contains case when then else end exists count on call yield drop
```

총 43개.

### 2.3 의도적으로 예약하지 않은 단어 (`lexer.rs:24-27`)

```
INDEX  CONSTRAINT  FOR  REQUIRE  UNIQUE  OPTIONS  EACH  IF
VECTOR  FULLTEXT  RANGE  TEXT  POINT  KEY
```

이들은 DDL에서만 등장하고, 파서가 **철자로 매칭**한다 (`parser.rs:245-280`의 `word_at` / `at_word` / `eat_word`).
예약해 버리면 `text`라는 프로퍼티나 `Range`라는 라벨을 쓸 수 없게 되기 때문이다.

### 2.4 구두점 (`lexer.rs:229-251`)

| 길이 | 토큰 |
|---|---|
| 3자 | `<->` |
| 2자 | `<=` `>=` `<>` `!=` `->` `<-` `..` `=~` `+=` `\|\|` |
| 1자 | `( ) [ ] { } < > = + - * / % ^ . , : ; \|` |

가장 긴 것부터 시도하므로 `<=`가 `<` + `=`로 쪼개지지 않는다.

### 2.5 유니코드

식별자 스캔은 바이트가 아니라 문자 단위다 (`lexer.rs:127-153`). 주석에 이유가 있다:

> A graph whose classes are named in Korean — `(:회의실)` — is an ordinary graph,
> and Neo4j accepts those names.

문자열 리터럴도 멀티바이트 UTF-8을 그대로 보존한다 (`lexer.rs:186-195`, `utf8_len` `lexer.rs:254-264`).
단위 테스트가 이를 고정한다 (`lexer.rs:290-294`, `RETURN '한글 테스트'`).

### 2.6 주석

`//` 줄 주석과 `/* */` 블록 주석을 공백으로 스킵한다 (`lexer.rs:54-79`).

---

## 3. 사실 — 파서 (`cypher/parser.rs`, 1,177줄)

재귀 하강. 백트래킹은 `eat_if_exists`(`parser.rs:283-297`) 한 곳만 인덱스를 되감는다.

### 3.1 절(clause) 커버리지

`parser.rs:124-169` `parse_query()`:

| 절 | 지원 | 파서 진입점 | AST |
|---|---|---|---|
| `MATCH` | ✅ | `parse_match` `parser.rs:171` | `Clause::Match` |
| `OPTIONAL MATCH` | ✅ | `parser.rs:130-136` | `Clause::Match { optional: true }` |
| `WHERE` (MATCH/WITH 뒤) | ✅ | `parse_match_body` `parser.rs:181` | |
| `UNWIND … AS x` | ✅ | `parse_unwind` `parser.rs:185` | `Clause::Unwind` |
| `WITH … [WHERE]` | ✅ | `parse_with` `parser.rs:193` | `Clause::With` |
| `RETURN` | ✅ | `parse_return` `parser.rs:200` | `Clause::Return` |
| `CREATE` (패턴) | ✅ | `parse_create` `parser.rs:511` | `Clause::Create` |
| `MERGE … ON CREATE/MATCH SET` | ✅ | `parse_merge` `parser.rs:523` | `Clause::Merge` |
| `SET` | ✅ | `parse_set` `parser.rs:543` | `Clause::Set` |
| `REMOVE` | ✅ | `parse_remove` `parser.rs:548` | `Clause::Remove` |
| `DELETE` / `DETACH DELETE` | ✅ | `parse_delete` `parser.rs:600` | `Clause::Delete` |
| `CALL … YIELD` | ✅ | `parse_call` `parser.rs:303` | `Clause::Call` |
| `CREATE INDEX/CONSTRAINT`, `DROP …` | ✅ | `parse_ddl_create` `parser.rs:352`, `parse_drop` `parser.rs:498` | `Clause::Ddl` |
| `UNION` / `UNION ALL` | **지원** | `parser.rs:161-166` → `Query.union` → `compile.rs` `compile_read` | 분기마다 서브쿼리로 감싸 `UNION [ALL]` 로 잇는다. 컬럼 이름·순서가 다르면 오류. [수정 경위](12_fixed_correctness.md#1-union--arch-01) |
| `FOREACH` | ❌ | — | 파싱 오류 (`parser.rs:1167-1171` 테스트) |
| `CALL { … }` (서브쿼리) | ❌ | — | 파싱 오류 |
| `LOAD CSV` | ❌ | — | 파싱 오류 |

> **`UNION`은 파싱되지만 실행되지 않는다.** `ast::Query.union` 필드(`ast.rs:213-214`)는
> 채워지지만 `Compiler::compile_read`(`compile.rs:351-370`)는 `q.clauses`만 읽고 `union`을 보지 않는다.
> 브리핑 7절 스펙 상태표의 "003 working (`UNION` 미구현)"과 일치한다.

### 3.2 패턴 커버리지

`parse_pattern` `parser.rs:614-637`, `parse_node_pat` `parser.rs:639-657`, `parse_rel_pat` `parser.rs:659-724`.

| 문법 | 지원 | 근거 |
|---|---|---|
| `(n)` `(n:Label)` `(:Label)` `()` | ✅ | `parser.rs:641-653` |
| `(n:A:B)` 다중 라벨 | ✅ | `parser.rs:647-653` |
| `(n:A\|B)` 라벨 대안 | ✅ (파싱) | `parser.rs:650-652` — `labels` 벡터에 평탄화 |
| `(n {k: v})` 인라인 프로퍼티 | ✅ | `parse_prop_map` `parser.rs:726` |
| `-[r]->` `<-[r]-` `-[r]-` | ✅ | `parser.rs:659-724` |
| `-[:A\|B]->` 다중 타입 | ✅ | `parser.rs:678-684` (`\|` 뒤 `:` 선택적) |
| `-[r:T {k:v}]->` | ✅ | `parser.rs:706-708` |
| `-[*]->` `-[*2]->` `-[*1..3]->` `-[*..5]->` | ✅ | `parser.rs:685-705` |
| `p = (a)-[r]->(b)` 경로 변수 | ✅ | `parser.rs:616-623` |
| `shortestPath(...)` / `allShortestPaths(...)` | ❌ | — |
| `<->` (양방향 화살표) | 렉서만 | `lexer.rs:230` — `parse_rel_pat`는 `<-` / `-` / `->`만 본다 |

**가변 길이 기본 상한**: `*..`처럼 최대값을 안 쓰면 `MAX_VAR_LENGTH = 8`이 들어간다
(`parser.rs:697` → `cypher/mod.rs:24`).

**라벨 대안의 함정**: `(n:A|B)`는 `labels = ["A","B"]`로 평탄화되는데,
컴파일러의 `resolve_label_set`(`catalog/types.rs:152-189`)은 이를 **교집합**으로 해석한다
(가장 구체적인 하나를 고르고, 없으면 `LabelMatch::Nothing`). Cypher의 `|`는 합집합이므로
의미가 다르다 → `CODE-13`.

### 3.3 표현식 커버리지

우선순위 사슬 (`parser.rs:748-901`):

```
parse_expr
 └ parse_or        OR
    └ parse_xor    XOR
       └ parse_and AND
          └ parse_not         NOT (전위, 우결합)
             └ parse_comparison  = <> != <= >= < > =~ IN
                                 STARTS WITH / ENDS WITH / CONTAINS / IS [NOT] NULL
                └ parse_additive     + - ||
                   └ parse_multiplicative  * / %
                      └ parse_power         ^ (우결합)
                         └ parse_unary      - +
                            └ parse_postfix  .prop  [index]
                               └ parse_primary
```

`parse_primary`(`parser.rs:903-1054`)가 인식하는 것:

| 형태 | 근거 |
|---|---|
| 정수/실수/문자열/`$param`/`true`/`false`/`null` | `parser.rs:905-932` |
| `CASE [x] WHEN … THEN … ELSE … END` | `parse_case` `parser.rs:1118` |
| `count(...)` / `exists(...)` — 괄호 없으면 그냥 이름 | `parser.rs:938-946` |
| `( expr )` | `parser.rs:947-952` |
| `[a, b, c]` 리스트 | `parser.rs:976-986` |
| `[x IN xs WHERE p \| e]` 리스트 컴프리헨션 | `parser.rs:955-975` |
| `{k: v}` 맵 | `parser.rs:988-991` |
| `any/all/none/single(x IN xs WHERE p)` | `parser.rs:994-1009`, `list_pred_ahead` `parser.rs:1057` |
| `f(...)`, `ns.f(...)`, `ns1.ns2.f(...)` | `parser.rs:1010-1041` |
| `x { .a, .*, k: e }` 맵 프로젝션 | `parse_map_projection` `parser.rs:1074` |
| `xs[i]` 인덱싱 | `parser.rs:891-895` → `Expr::Func { name: "element_at" }` |

**네임스페이스 함수 이름의 모호성 해소** (`parser.rs:1012-1039`):
`a.b.c`가 함수 이름인지 프로퍼티 경로인지는 끝에 `(`가 오는지로만 알 수 있다.
그래서 먼저 체인 길이를 **재기만** 하고, `(`에 도달했을 때만 소비한다.
`MAX_NAMESPACE_DEPTH = 3`(`parser.rs:12`)이 상한이다 — `genai.vector.encode`가 2단계를 쓴다.

### 3.4 DDL 문법

`create_is_ddl`(`parser.rs:341-350`)이 `CREATE` 다음 단어로 패턴인지 DDL인지 결정한다.

지원 형태:

```cypher
CREATE [VECTOR|FULLTEXT|TEXT|POINT|RANGE] INDEX [name] [IF NOT EXISTS]
  FOR (n:Label) | FOR ()-[r:TYPE]-()
  ON (n.a, n.b) | ON EACH [n.a, n.b] | ON n.a
  [OPTIONS { ... }]

CREATE CONSTRAINT [name] [IF NOT EXISTS]
  FOR (n:Label)
  REQUIRE|ASSERT (n.a, n.b) IS UNIQUE | IS NOT NULL | IS NODE KEY | IS RELATIONSHIP KEY

DROP INDEX|CONSTRAINT name [IF EXISTS]
```

- `REQUIRE`(Neo4j 5)와 `ASSERT`(Neo4j 4) 둘 다 받는다 (`parser.rs:408-411`).
- `OPTIONS`는 **평탄화**된다 (`parse_ddl_options` `parser.rs:483-496`) —
  `OPTIONS {indexConfig: {`vector.dimensions`: 1536}}`에서 호출자는 그냥 `vector.dimensions`를 읽는다.
- 속성 목록은 세 가지 철자를 모두 받는다 (`parse_ddl_props` `parser.rs:455-479`).

### 3.5 오류 형태

`Parser::err`(`parser.rs:97-101`):

```rust
format!("{msg} at offset {pos}, near: …{snippet}…")
```

`snippet`은 오프셋 기준 앞 12자 / 총 36자다. 예:

```
expected '->' or '-' to close the relationship pattern at offset 21, near: …TCH (a)-[:R (b) RETURN a…
```

모듈 주석(`parser.rs:1-4`)이 정책을 명시한다:

> Unsupported syntax fails loudly at parse time with the offending construct named (FR-008):
> silently reinterpreting a query is worse than rejecting it.

---

## 4. 사실 — AST (`cypher/ast.rs`, 263줄)

### 4.1 구조

```
Query { clauses: Vec<Clause>, union: Option<(bool, Box<Query>)> }
  Clause::Match { patterns: Vec<Pattern>, optional: bool, where_: Option<Expr> }
  Clause::Unwind { expr, alias }
  Clause::With { proj: Projection, where_: Option<Expr> }
  Clause::Return(Projection)
  Clause::Create(Vec<Pattern>)
  Clause::Merge { pattern, on_create: Vec<SetOp>, on_match: Vec<SetOp> }
  Clause::Set(Vec<SetOp>) / Clause::Remove(Vec<SetOp>)
  Clause::Delete { exprs, detach }
  Clause::Call { name, args, yields: Vec<(String, String)> }
  Clause::Ddl(Ddl)

Pattern { path_var: Option<String>, elems: Vec<PatElem> }
  PatElem::Node(NodePat { var, labels, props })
  PatElem::Rel(RelPat  { var, types, dir, props, range: Option<(u32,u32)> })
```

`Projection`(`ast.rs:134-141`)은 `items` + `distinct` + `order` + `skip` + `limit`을 한 덩어리로 들고 있다.
`RETURN`과 `WITH`가 같은 타입을 쓴다 — 그래서 컴파일러가 `build_core` 하나로 둘 다 처리할 수 있다.

### 4.2 컴파일러가 실제로 읽는 두 개의 헬퍼

**`Expr::is_aggregate()`** (`ast.rs:218-237`) — 재귀적으로 집계 함수 존재를 판정한다.
집계 목록: `count sum avg min max collect stdev percentile`.

이 함수가 세 곳에서 의미를 갖는다:
1. `build_core`의 `GROUP BY` 자동 생성 (`compile.rs:481,495`)
2. `multiplicity_blind`의 도달성 재작성 판정 (`compile.rs:348`)
3. 쓰기 경로의 집계 접기 (`cypher/mod.rs:306,329`)

> **주의**: `percentile`은 `is_aggregate()` 목록에 있지만 `Compiler::func`(`compile.rs:1415-1568`)에는
> 구현이 없다. `percentile_cont(...)`을 쓰면 `unknown function 'percentile...'`로 거절된다.

**`Expr::default_alias()`** (`ast.rs:241-262`) — 별칭 없는 `RETURN` 항목의 컬럼 이름을 만든다.
`Expr::Prop(Var("w"), "title")` → `"w.title"`. 이 문자열이 그대로
`jsonb_build_object('w.title', …)`의 키가 되고 (`compile.rs:607`),
Bolt의 `fields` 배열이 된다 (`cypher/mod.rs:734-737` → `bolt/src/session.rs:283-289`).

---

## 5. 사실 — 단위 테스트

`cypher/lexer.rs:266-302` — 4개:
`basic_pattern`, `strings_and_params`, `unicode_strings_survive`, `numbers`.

`cypher/parser.rs:1133-1177` — 6개:
`simple_match_return`, `var_length`, `direction_parsing`, `aggregates_detected`,
`rejects_unknown_clause_loudly`, `string_predicates`.

`cargo test`로 돈다. 데이터베이스가 필요 없다.

---

## 금지 / 필수

- **금지**: `KEYWORDS`(`lexer.rs:17-23`)에 단어를 추가하기 전에, 그 단어가 라벨/관계 타입/프로퍼티
  이름으로 쓰일 수 있는지 확인하지 않는 것. 추가하면 그 이름을 쓰는 기존 그래프가 깨진다.
  DDL 전용 단어는 `eat_word`로 철자 매칭한다.
- **금지**: 파서가 모르는 문법을 **조용히 무시**하는 것. 반드시 `Err`를 낸다 (`parser.rs:1-4`).
- **금지**: `Tok::Keyword`의 소문자 철자를 이름으로 쓰는 것. 이름 자리에서는 `Token.raw`를 쓴다
  (`parser.rs:103-118`).
- **필수**: 새 절이나 표현식을 추가하면 `ast.rs`의 `is_aggregate()`와 `default_alias()`가
  그것을 어떻게 취급하는지 결정한다. 기본값(`_ => false` / `_ => "expr"`)에 기대지 않는다.
- **필수**: 새 파싱 규칙에는 `parser.rs` 하단 `#[cfg(test)]`에 테스트를 추가한다.

<!-- affects: backend, api -->
<!-- requires-update: 02_api/02_cypher_api.md -->
