# Cypher API — 진입 함수와 문법 경계

> **이 문서가 답하는 질문**
> - Cypher는 어떤 함수로 들어가고, 각각 무엇이 다른가?
> - 지원되는 절·표현식·함수는 정확히 무엇인가?
> - 지원되지 않는 것은 어떻게 **보고**되는가? (거부 / 조용한 무시)
> - 파라미터는 왜 주입이 구조적으로 불가능한가?
> - 가변 길이 매치는 언제 `og_vlp`이고 언제 `og_reach`인가?

---

## 1. 결정(Decision) — Cypher는 함수 호출로 진입한다

PostgreSQL 16에는 최상위 파서를 대체하는 훅이 없다. 헌법 원칙 I(포크 금지)가
원칙 II보다 우선하므로, Cypher는 SQL 함수 인자로 들어온다
(README 스펙 표, [engine/src/cypher/mod.rs:1](../../engine/src/cypher/mod.rs#L1)).

파이프라인: **lex → parse → AST → SQL 컴파일 → PostgreSQL 실행**.

읽기 질의는 **SQL 문 하나**가 된다. 그래서 조인 순서·스캔 선택·병렬성을 플래너가
소유한다. 쓰기 질의는 읽기 부분이 만든 바인딩 위에서 절차적으로 실행되며,
호출자의 트랜잭션 안에서 돈다([engine/src/cypher/mod.rs:5](../../engine/src/cypher/mod.rs#L5)).

**컴파일 캐시**: `(graph, query)` → `(sql, columns)`를 백엔드 로컬
`thread_local` HashMap에 캐시한다. 512개를 넘으면 통째로 비운다
([engine/src/cypher/mod.rs:26](../../engine/src/cypher/mod.rs#L26),
[:58](../../engine/src/cypher/mod.rs#L58)). 실행 계획 캐시는 PostgreSQL이 따로 한다.

---

## 2. 진입 함수

### `og_cypher(graph text, query text, params jsonb DEFAULT '{}') RETURNS SETOF jsonb`

정의: [engine/src/cypher/mod.rs:83](../../engine/src/cypher/mod.rs#L83) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값(`PARALLEL UNSAFE`)

**무엇을 하는가**: Cypher 질의를 실행하고 결과 행마다 jsonb 객체 하나를 반환한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | Cypher 질의 텍스트 |
| `params` | `jsonb` | 선택 | `'{}'` | `$name` 파라미터 바인딩. 값은 절대 텍스트로 보간되지 않음 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `og_cypher` | `jsonb` | 예 | 결과 행 하나. `RETURN` 별칭이 키가 된다 |

**노드/엣지 JSON 모양** ([engine/sql/access.sql:231](../../engine/sql/access.sql#L231), [:259](../../engine/sql/access.sql#L259)):

```json
{"_id": 412316860417, "_type": "Person", "name": "Aria", "born": 1990}
{"_id": 549755813889, "_type": "ACTED_IN", "_src": 412316860417, "_dst": 481036337153, "role": "Neo"}
```

**부수 효과**
- 호출마다 `crate::stats::reset()` — `og_cypher_stats()`가 읽는 카운터를 0으로.
- 성공·실패 모두 `og_data.og_audit`에 한 행을 남긴다
  ([engine/src/cypher/mod.rs:122](../../engine/src/cypher/mod.rs#L122)).
  `query` 컬럼은 `[<graph>] <query>` 형태로 저장되고, `error_code` 컬럼에는
  오류 메시지 앞 200자가 들어간다.

**예제**

```sql
SELECT og_cypher('default', $$
  MATCH (p:Person)-[r:ACTED_IN]->(w:Work)
  RETURN p.name AS actor, r.role AS role, w.title AS title
  ORDER BY title
$$);
```

파라미터 사용 ([docs/cypher.md:103](../../docs/cypher.md)):

```sql
SELECT og_cypher('kb',
  $$ MATCH (p:Person) WHERE p.age > $min AND p.city = $city RETURN p.name $$,
  '{"min": 30, "city": "Seoul"}'::jsonb);
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 파싱 실패 | `cypher parse error: <parser message>` ([mod.rs:97](../../engine/src/cypher/mod.rs#L97)) |
| 컴파일 실패 | `cypher error: <compile message>` ([mod.rs:140](../../engine/src/cypher/mod.rs#L140)) |
| 실행 실패 | `cypher execution failed: <pg error>` + 컴파일된 SQL 전문 첨부 ([mod.rs:149](../../engine/src/cypher/mod.rs#L149)) |
| 그래프 없음 | `graph '<g>' does not exist` |

> ⚠️ **`jsonb`는 키를 정렬한다.** 반환된 행 객체만으로는 `RETURN`이 요구한 컬럼
> 순서를 알 수 없다. 순서가 필요하면 `og_cypher_columns()`를 쓸 것(§2.6).

---

### `og_cypher_json(graph text, query text, params jsonb DEFAULT '{}') RETURNS jsonb`

정의: [engine/src/interop/mod.rs:35](../../engine/src/interop/mod.rs#L35) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: `og_cypher`와 같은 결과를 **배열 하나**로 묶어 반환한다. PostgREST / supabase-js RPC 진입점.

**인자**: `og_cypher`와 동일.

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `jsonb` | 아니오 | 항상 JSON 배열. 결과가 없으면 `[]` |

**예제**

```sql
SELECT og_cypher_json('default', 'MATCH (p:Person) RETURN p.name AS name LIMIT 2');
-- [{"name": "Aria"}, {"name": "Bo"}]
```

**실패 조건**: 내부적으로 `og_cypher`를 호출하므로 동일. SPI 오류는 메시지가
그대로 전달된다(`error!("{e}")`, [interop/mod.rs:44](../../engine/src/interop/mod.rs#L44)).

---

### `og_cypher_sql(graph text, query text) RETURNS text`

정의: [engine/src/cypher/mod.rs:74](../../engine/src/cypher/mod.rs#L74) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: Cypher가 컴파일된 SQL 문 자체를 문자열로 돌려준다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | **읽기** 질의만 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `text` | 아니오 | `$1`을 jsonb 파라미터로 받는 완전한 `SELECT` 문 |

**예제**

```sql
SELECT og_cypher_sql('default', 'MATCH (p:Person) RETURN p.name AS name');
```

출력된 SQL은 그대로 `EXPLAIN` 하거나 자신의 질의에 끼워 넣을 수 있다.
`$1` 자리에 파라미터 jsonb를 바인딩해야 한다.

**실패 조건**
- 쓰기 질의 → `write queries are not compiled to a single statement`
  ([mod.rs:54](../../engine/src/cypher/mod.rs#L54))
- 파싱/컴파일 실패 → 해당 메시지 그대로

---

### `og_cypher_explain(graph text, query text, analyze bool DEFAULT false) RETURNS jsonb`

정의: [engine/src/cypher/mod.rs:676](../../engine/src/cypher/mod.rs#L676) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 컴파일된 SQL과 PostgreSQL의 실행 계획을 한 번에 준다(spec 003 FR-017).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `query` | `text` | 필수 | — | 읽기 질의 |
| `analyze` | `bool` | 선택 | `false` | `true`면 `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` — **질의를 실제로 실행한다** |

**반환**

| 키 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `columns` | `array<text>` | 아니오 | 컴파일러가 계산한 결과 컬럼 이름 |
| `sql` | `text` | 아니오 | 컴파일된 SQL |
| `plan` | `jsonb` | 예 | `EXPLAIN (FORMAT JSON)` 출력. 실패 시 `null` |

**예제**

```sql
SELECT jsonb_pretty(og_cypher_explain('default',
  'MATCH (p:Person)-[:ACTED_IN]->(w:Work) RETURN w.title', true));
```

**실패 조건**: `og_cypher_sql`과 동일 + `EXPLAIN` 자체가 실패하면 `plan`이 `null`이 된다(오류는 나지 않음).

> ⚠️ `analyze => true`는 **쓰기가 아닌 질의여도 실제 실행**한다. 큰 질의에는
> 먼저 `og_estimate()`를 쓸 것([07_agent_interface.md](07_agent_interface.md)).

---

### `og_cypher_check(query text) RETURNS jsonb`

정의: [engine/src/cypher/mod.rs:699](../../engine/src/cypher/mod.rs#L699) · 휘발성: `IMMUTABLE` · 병렬: `PARALLEL SAFE`

**무엇을 하는가**: **파싱만** 한다. 데이터베이스에 접근하지 않으므로 그래프 인자도 없다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `query` | `text` | 필수 | — | Cypher 질의 |

**반환**

| 키 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `ok` | `bool` | 아니오 | 파싱 성공 여부 |
| `clauses` | `int` | `ok=false`면 없음 | 절 개수 |
| `write` | `bool` | `ok=false`면 없음 | `CREATE`/`MERGE`/`SET`/`REMOVE`/`DELETE`/DDL 중 하나라도 있으면 `true` |
| `error` | `text` | `ok=true`면 없음 | 파서 메시지 |

**예제**

```sql
SELECT og_cypher_check('MATCH (a) WITH a RETURN a');
-- {"ok": true, "clauses": 3, "write": false}

SELECT og_cypher_check('MATCH (a) RETRUN a');
-- {"ok": false, "error": "…: expected a clause keyword"}
```

**실패 조건**: 없음 — 오류를 던지지 않고 `ok: false`로 보고한다.

> ⚠️ `ok: true`는 **컴파일 가능**을 뜻하지 않는다. 파싱만 확인한다.
> `UNION`이 붙은 질의도 `ok: true`가 되지만 실행 결과는 첫 절반뿐이다(§4.1).

**용도**: Bolt 게이트웨이가 읽기/쓰기 판정에 이 함수를 쓴다
([bolt/src/session.rs:444](../../bolt/src/session.rs#L444)). 키워드 스캔을 하지 않는
이유는 문자열 리터럴 안의 `CREATE`가 쓰기가 아니기 때문이다.

---

### `og_cypher_columns(query text) RETURNS text[]`

정의: [engine/src/cypher/mod.rs:717](../../engine/src/cypher/mod.rs#L717) · 휘발성: `IMMUTABLE` · 병렬: `PARALLEL SAFE`

**무엇을 하는가**: `RETURN` 순서대로의 결과 컬럼 이름을 반환한다. 파싱만 하며 DB에 접근하지 않는다.

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `text[]` | 아니오 | `RETURN`이 없거나 `RETURN *`이면 **빈 배열** |

`RETURN *`에서 빈 배열을 주는 이유: 패턴이 바인딩한 변수는 파서만으로는 알 수
없다. 틀린 순서를 주느니 아무것도 주지 않고, 호출자가 첫 행의 키로 폴백한다
([mod.rs:723](../../engine/src/cypher/mod.rs#L723) 주석).

**별칭 없는 항목의 이름 규칙** ([engine/src/cypher/ast.rs:241](../../engine/src/cypher/ast.rs#L241) `default_alias`):

| 표현식 | 컬럼 이름 |
|---|---|
| `n` | `n` |
| `n.name` | `n.name` |
| `count(*)` | `count(*)` |
| `'x'` | `"x"` (따옴표 포함) |
| `$p` | `$p` |
| 그 외 | `expr` |

**예제**

```sql
SELECT og_cypher_columns('MATCH (p:Person) RETURN p.name AS actor, count(*)');
-- {actor,"count(*)"}
SELECT og_cypher_columns('MATCH (p) RETURN *');
-- {}
```

**실패 조건**: 파싱 실패 시 파서 메시지로 `ERROR`. (`og_cypher_check`와 달리
오류를 던진다 → [12_improvements_api.md](12_improvements_api.md) API-08.)

---

### `og_cypher_stats() RETURNS jsonb`

정의: [engine/src/cypher/mod.rs:117](../../engine/src/cypher/mod.rs#L117) · 휘발성: `VOLATILE` · 병렬: `PARALLEL UNSAFE`

**무엇을 하는가**: **같은 커넥션의 직전 `og_cypher()` 호출**이 바꾼 것을 Neo4j 카운터 이름으로 반환한다.

**인자**: 없음.

**반환** ([engine/src/stats.rs:50](../../engine/src/stats.rs#L50)) — 항상 아래 11개 키:

| 키 | 타입 | 설명 |
|---|---|---|
| `nodes-created` | int | |
| `nodes-deleted` | int | |
| `relationships-created` | int | |
| `relationships-deleted` | int | |
| `properties-set` | int | 프로퍼티 **대입** 횟수 (같은 키를 두 번 쓰면 2) |
| `labels-added` | int | 여기선 노드 생성과 1:1 |
| `indexes-added` / `indexes-removed` | int | Cypher DDL 경로 |
| `constraints-added` / `constraints-removed` | int | Cypher DDL 경로 |
| `contains-updates` | bool | 위 10개 합이 0보다 크면 `true` |

**필수 사용 규칙 (계약)**
- 반드시 **같은 커넥션**에서,
- 반드시 **다음 `og_cypher()` 호출 이전에** 물어야 한다.
- 이 상태는 **트랜잭션 로그가 아니다** — 롤백된 문의 카운트도 남고, 다음
  `og_cypher()` 호출이 초기화한다([engine/src/stats.rs:9](../../engine/src/stats.rs#L9) 주석).

**예제**

```sql
SELECT og_cypher('default', $$ CREATE (:Person {name:'Zed'}) $$);
SELECT og_cypher_stats();
-- {"nodes-created": 1, "labels-added": 1, "properties-set": 1,
--  "contains-updates": true, …}
```

Bolt 게이트웨이가 쓰기 완료 후 요약을 쓰기 직전 이 함수를 부른다
([bolt/src/session.rs:432](../../bolt/src/session.rs#L432)).

---

## 3. 지원되는 Cypher — 사실

원문 근거: [docs/cypher.md](../../docs/cypher.md).
아래는 파서·컴파일러 코드에서 재확인한 목록이다.

### 3.1 절(Clause)

파서가 받아들이는 절 키워드 ([engine/src/cypher/parser.rs:130](../../engine/src/cypher/parser.rs#L130)):
`MATCH`, `OPTIONAL MATCH`, `WHERE`, `RETURN`, `WITH`, `UNWIND`, `CREATE`,
`MERGE`, `SET`, `REMOVE`, `DELETE` / `DETACH DELETE`, `CALL … YIELD`,
`CREATE/DROP INDEX`, `CREATE/DROP CONSTRAINT`.

| 절 | 컴파일 결과 | 위치 |
|---|---|---|
| `MATCH` | `CROSS JOIN LATERAL` 연쇄 | [compile.rs:165](../../engine/src/cypher/compile.rs#L165) |
| `OPTIONAL MATCH` | `LEFT JOIN LATERAL` + 술어를 `ON`에 배치 | [compile.rs:129](../../engine/src/cypher/compile.rs#L129) |
| `WITH` | 지금까지를 서브쿼리로 감싸고 투영 컬럼만 남김 = "질의 지평선" | [compile.rs:382](../../engine/src/cypher/compile.rs#L382) |
| `RETURN` | 최종 `SELECT` (`DISTINCT`, `ORDER BY`, `SKIP`, `LIMIT` 포함) | [compile.rs:368](../../engine/src/cypher/compile.rs#L368) |
| `UNWIND` | 리스트 전개 | [compile.rs:198](../../engine/src/cypher/compile.rs#L198) |
| `CALL … YIELD` | 프로시저를 `FROM`에 놓는 관계로 컴파일 | [compile.rs:410](../../engine/src/cypher/compile.rs#L410) |

**필수 규칙**: 읽기 질의는 반드시 `RETURN`으로 끝나야 한다 —
`a read query must end with RETURN` ([compile.rs:368](../../engine/src/cypher/compile.rs#L368)).

### 3.2 패턴

```cypher
(a)                          -- any node
(a:Person)                   -- label; matches every subtype
(a:Person {name: 'Aria'})    -- inline property filter
(a)-[r:KNOWS]->(b)           -- directed, typed, bound
(a)<-[:KNOWS]-(b)            -- reverse
(a)-[:KNOWS]-(b)             -- either direction
(a)-[:KNOWS|FOLLOWS]->(b)    -- type alternatives
(a)-[:KNOWS*1..3]->(b)       -- variable length, trail semantics
p = (a)-[:KNOWS]->(b)        -- path variable
```

- 라벨은 **타입과 그 모든 서브타입**에 매치된다. 컴파일 시점에 구간 인덱스로 한 번
  펼쳐지므로 행당 비용이 없다([compile.rs:826](../../engine/src/cypher/compile.rs#L826)).
- 다중 라벨 `(a:A:B)`는 **가장 구체적인** 라벨로 해석된다. 두 라벨이 한 상속
  체인에 없으면 아무것도 매치하지 않는다
  ([engine/src/catalog/types.rs:152](../../engine/src/catalog/types.rs#L152)).
- **상한 없는 `*..`는 8홉으로 잘린다** — `MAX_VAR_LENGTH = 8`
  ([engine/src/cypher/mod.rs:24](../../engine/src/cypher/mod.rs#L24),
  [parser.rs:697](../../engine/src/cypher/parser.rs#L697)).

### 3.3 표현식

```
literals      42   3.14   'text'   "text"   true   false   null   [1,2]   {k: v}
parameters    $name                                (bound, never interpolated)
properties    a.name    r.since
comparison    =  <>  !=  <  <=  >  >=   IS NULL   IS NOT NULL   IN
string        STARTS WITH   ENDS WITH   CONTAINS   =~
boolean       AND  OR  XOR  NOT
arithmetic    +  -  *  /  %  ^  ||
conditional   CASE … WHEN … THEN … ELSE … END
list comp     [x IN xs WHERE p | e]
list pred     any / all / none / single (x IN xs WHERE p)
map proj      n { .name, .age, total: count(x), .* }
```

AST 근거: [engine/src/cypher/ast.rs:73](../../engine/src/cypher/ast.rs#L73) `Expr`.

### 3.4 함수 — 코드에서 확인한 전체 목록

정의: [engine/src/cypher/compile.rs:1415](../../engine/src/cypher/compile.rs#L1415) `func()`

| 그룹 | 함수 | 컴파일 결과 |
|---|---|---|
| 집계 | `count` (`count(*)`, `count(DISTINCT x)` 포함) | `count(*)` / `count(DISTINCT …)` |
| 집계 | `sum`, `avg`, `min`, `max` | 동명의 SQL 집계 |
| 집계 | `collect` | `jsonb_agg` |
| 집계 | `stdev` | `stddev_samp(…::float8)` |
| 그래프 | `id(v)` | 요소의 `int8` 식별자 |
| 그래프 | `elementId(v)` | 위의 `::text` |
| 그래프 | `labels(n)` | `og_supertypes()`를 이름으로 펼친 **jsonb 배열** — 상위 체인 전체 |
| 그래프 | `type(r)` | `og_type_name(...)` |
| 그래프 | `nodes(p)`, `relationships(p)` | 경로 표현식 그대로 |
| 리스트 | `length`, `size` | `jsonb_array_length` |
| 리스트 | `keys` | `to_jsonb(ARRAY(SELECT jsonb_object_keys(…)))` |
| 리스트 | `element_at(l, i)` | `(l -> (i)::int)` |
| 문자열 | `toUpper`/`upper`, `toLower`/`lower`, `trim` | `upper` / `lower` / `btrim` |
| 문자열 | `substring(s, start[, len])` | `substring(… from start+1 …)` — **0-기반 → 1-기반 보정** |
| 문자열 | `replace`, `split` | `replace` / `to_jsonb(string_to_array(…))` |
| 수치 | `abs`, `ceil`, `floor`, `round`, `sqrt`, `rand` | 동명 SQL 함수 / `random()` |
| 변환 | `toString`, `toInteger`, `toFloat` | `::text` / `::int8` / `::float8` |
| 시간 | `timestamp`, `datetime` | `extract(epoch from now())::int8` / `now()` |
| 기타 | `coalesce` | 타입이 알려진 분기에 맞춰 캐스트 후 `coalesce` |
| 기타 | `exists(x)` | `(x IS NOT NULL)` |
| 벡터 | `vector.similarity(a,b)` / `similarity(a,b)` | `1 - (a::vector <=> b::vector)` |
| 벡터 | `vector.distance(a,b)` | `a::vector <=> b::vector` |
| 벡터 | `vector.l2(a,b)` | `a::vector <-> b::vector` |
| 벡터 | `genai.vector.encode(text[, provider[, config]])` | `og_genai_encode(...)` — 기본 비활성, [09_neo4j_compat.md](09_neo4j_compat.md) 참조 |

**그 외의 함수는 이름으로 거부된다**:

```
ERROR:  unknown function 'shortestpath'. supported: count, sum, avg, min, max,
        collect, id, elementId, labels, type, length, size, toUpper, toLower,
        trim, substring, replace, split, coalesce, abs, ceil, floor, round,
        sqrt, rand, toString, toInteger, toFloat, timestamp, datetime, exists,
        keys, vector.similarity, vector.distance, genai.vector.encode
```

([compile.rs:1559](../../engine/src/cypher/compile.rs#L1559))

> 이 목록에 `elementId`, `element_at`, `stdev`, `nodes`, `relationships`가 실제로
> 지원되지만 **오류 메시지의 "supported:" 목록에는 일부가 빠져 있다**
> → [12_improvements_api.md](12_improvements_api.md) API-09.

### 3.5 쓰기 절

| 절 | 지원 범위 | 위치 |
|---|---|---|
| `CREATE` | 노드·관계. 새 요소당 라벨/타입 **정확히 하나** | [compile.rs / mod.rs run_write](../../engine/src/cypher/mod.rs#L152) |
| `MERGE` | `ON CREATE SET` / `ON MATCH SET` | [parser.rs:537](../../engine/src/cypher/parser.rs#L537) |
| `SET` | `a.prop = expr`, `a = {…}`, `a += {…}`, `a:Label` | [ast.rs:143](../../engine/src/cypher/ast.rs#L143) |
| `REMOVE` | `a.prop`, `a:Label` | [parser.rs:563](../../engine/src/cypher/parser.rs#L563) |
| `DELETE` / `DETACH DELETE` | 평문 `DELETE`는 관계가 남은 노드를 거부 | — |

쓰기 질의는 단일 SQL 문으로 컴파일되지 않는다. 읽기 부분(`MATCH`/`UNWIND`)이
바인딩을 만들고, 그 위에서 변경이 절차적으로 돈다
([engine/src/cypher/mod.rs:158](../../engine/src/cypher/mod.rs#L158)).

**필수 규칙**: 스키마 명령(`CREATE INDEX` 등)은 다른 절과 같은 질의에 섞을 수 없다 —
`a schema command cannot be combined with other clauses`
([mod.rs:163](../../engine/src/cypher/mod.rs#L163)).

---

## 4. 지원되지 않는 것 — **어떻게 보고되는가**

이 절이 이 문서에서 가장 중요하다. "지원 안 됨"은 세 가지 방식으로 나타나고,
그중 하나는 **조용하다**.

### 4.1 ⚠️ `UNION` — 파싱되지만 **조용히 버려진다**

- 파서는 `UNION` / `UNION ALL`을 읽어 `Query.union` 필드에 넣는다
  ([parser.rs:161](../../engine/src/cypher/parser.rs#L161), [ast.rs:213](../../engine/src/cypher/ast.rs#L213)).
- 컴파일러(`compile.rs`)에는 `union`을 참조하는 코드가 **한 줄도 없다** —
  `compile_read`는 `q.clauses`만 순회한다([compile.rs:351](../../engine/src/cypher/compile.rs#L351)).
- 결과: **오류 없이 첫 번째 절반의 결과만 반환된다.**
- `og_cypher_check()`도 `{"ok": true}`를 준다.

```sql
-- Returns ONLY the Person rows. The Company half is silently dropped.
SELECT og_cypher('default', $$
  MATCH (p:Person)  RETURN p.name AS name
  UNION
  MATCH (c:Company) RETURN c.name AS name
$$);
```

**우회 방법**: SQL 수준에서 합칠 것.

```sql
SELECT * FROM og_cypher('default', 'MATCH (p:Person) RETURN p.name AS name')
UNION
SELECT * FROM og_cypher('default', 'MATCH (c:Company) RETURN c.name AS name');
```

`docs/cypher.md`는 이를 "parsed, not compiled"로 표기하고 있으나
([docs/cypher.md:237](../../docs/cypher.md)), **조용히 결과를 잘라 먹는다**는 사실은
적혀 있지 않다 → [12_improvements_api.md](12_improvements_api.md) **API-01**.

### 4.2 파싱 단계에서 거부되는 것

| 구문 | 실제 오류 메시지 | 위치 |
|---|---|---|
| `FOREACH` | `expected a clause keyword` — `foreach`는 키워드 목록에 없어 식별자로 렉싱됨 ([lexer.rs:17](../../engine/src/cypher/lexer.rs#L17)) | [parser.rs:153](../../engine/src/cypher/parser.rs#L153) |
| 알 수 없는 절 키워드 | `unexpected clause '<KW>'` | [parser.rs:149](../../engine/src/cypher/parser.rs#L149) |
| 빈 질의 | `empty query` | [parser.rs:158](../../engine/src/cypher/parser.rs#L158) |
| 남는 입력 | `unexpected trailing input` | [parser.rs:93](../../engine/src/cypher/parser.rs#L93) |
| 잘못된 `SET` | ``SET expects `var.prop = …`, `var = {…}`, `var += {…}` or `var:Label` `` | [parser.rs:591](../../engine/src/cypher/parser.rs#L591) |
| 잘못된 `REMOVE` | `REMOVE expects a property or a label` | [parser.rs:563](../../engine/src/cypher/parser.rs#L563) |
| 관계 패턴 미완성 | `expected '->' or '-' to close the relationship pattern` | [parser.rs:720](../../engine/src/cypher/parser.rs#L720) |
| 제약 종류 오류 | `expected UNIQUE, NOT NULL or NODE KEY` | [parser.rs:423](../../engine/src/cypher/parser.rs#L423) |
| 패턴 컴프리헨션 | 파싱되지 않음 → `expected an expression` | [parser.rs:1052](../../engine/src/cypher/parser.rs#L1052) |

### 4.3 컴파일 단계에서 거부되는 것

| 상황 | 오류 메시지 | 위치 |
|---|---|---|
| 알 수 없는 함수 (`shortestPath` 포함) | `unknown function '<f>'. supported: …` | [compile.rs:1559](../../engine/src/cypher/compile.rs#L1559) |
| 미정의 변수 | `variable '<v>' is not defined in this query` | [compile.rs:1091](../../engine/src/cypher/compile.rs#L1091) |
| 패턴 변수 아닌 것에 프로퍼티 접근 | `property access is only supported on pattern variables` | [compile.rs:1178](../../engine/src/cypher/compile.rs#L1178) |
| 경로 변수에 `.prop` | `'<v>' is a path; use length(<v>) or nodes(<v>)` | [compile.rs:1188](../../engine/src/cypher/compile.rs#L1188) |
| `id()`/`labels()`/`type()`에 비패턴 인자 | `id() expects a pattern variable` 등 | [compile.rs:1451](../../engine/src/cypher/compile.rs#L1451) |
| 읽기 질의에 쓰기 절 | `this clause is only valid in a write query` | [compile.rs:364](../../engine/src/cypher/compile.rs#L364) |
| `RETURN` 없음 | `a read query must end with RETURN` | [compile.rs:368](../../engine/src/cypher/compile.rs#L368) |
| 등록되지 않은 프로시저 | `procedure '<p>' is not available. supported: …` | [compat/procs.rs:154](../../engine/src/compat/procs.rs#L154) |
| 한 체인에 없는 라벨 조합으로 `CREATE` | `no node can carry the labels (:A:B) at once — …` | [catalog/types.rs:224](../../engine/src/catalog/types.rs#L224) |

### 4.4 ⚠️ 존재하지 않는 라벨 — 오류가 아니라 **NOTICE**

```sql
SELECT og_cypher('default', 'MATCH (p:Persn) RETURN p');
-- NOTICE:  label 'Persn' does not exist in graph 'default' — matching nothing.
--          did you mean: Person
--  og_cypher
-- -----------
-- (0 rows)
```

정의: [engine/src/catalog/types.rs:160](../../engine/src/catalog/types.rs#L160).
Cypher에서 존재하지 않는 라벨은 "아무것도 매치하지 않음"이며 오류가 아니다.
라벨을 만들기 전에 탐색해 보는 호출자가 이 동작에 의존한다.

> `docs/cypher.md:295`은 이를 `ERROR: unknown label 'Persn' in graph 'social'.
> did you mean: Person` 이라고 적고 있으나, **코드에는 그런 오류가 존재하지 않는다.**
> `og_explain_error()`의 `UNKNOWN_LABEL` 코드도 그래서 도달 불가다
> → [11_errors.md](11_errors.md), [12_improvements_api.md](12_improvements_api.md) **API-10**.

### 4.5 미구현으로 명시된 것 (원문: [docs/cypher.md:234](../../docs/cypher.md))

| 구문 | 실제 동작 | 대안 |
|---|---|---|
| `UNION` | 파싱되나 컴파일 안 됨 → **조용히 절반만** (§4.1) | SQL `UNION` |
| `FOREACH` | 파싱 거부 | `UNWIND` + 쓰기 절 |
| `SET a:Label` 로 **새 라벨 추가** | 이유와 함께 거부 | 타입은 식별자의 일부. 클래스 이름 변경은 `REMOVE n:Old SET n:New` |
| 패턴 컴프리헨션 | 파싱 안 됨 | `collect()` + 서브쿼리 |
| `shortestPath` | `unknown function` | `og_vlp()` 를 depth 순 정렬, 또는 `og_csr_hops()` |
| 사용자 정의 프로시저 | 기구 자체가 없음 | `og_*` SQL 함수 표면 |
| SPARQL | 미구현 (spec 006 partial) | [08_interop_and_rdf.md](08_interop_and_rdf.md) |

---

## 5. 결정(Decision) — 가변 길이 매치가 컴파일되는 두 방식

`(a)-[:K*1..6]->(b)`는 두 SQL 함수 중 하나로 컴파일된다
([compile.rs:865](../../engine/src/cypher/compile.rs#L865)).

| 컴파일 대상 | 의미론 | 언제 |
|---|---|---|
| `og_vlp` | **트레일 열거** — 경로 1개당 1행, `path int8[]` 바인딩 가능 | 기본값 |
| `og_reach` | **도달성** — 노드 1개당 1행, 최초 도달 깊이 | 세 조건이 **모두** 참일 때 |

`og_reach`로 컴파일되는 세 조건:

1. `rel.var.is_none()` — 관계 변수를 바인딩하지 않음.
2. `self.reachability_only` — 질의가 경로 **다중도(multiplicity)** 를 관측할 수
   없음 (`Compiler::multiplicity_blind`, [compile.rs:339](../../engine/src/cypher/compile.rs#L339)):
   - `WITH`가 하나라도 있으면 **실격**.
   - `RETURN DISTINCT …`이면 통과.
   - 아니면 투영이 집계여야 하고, 모든 집계가 중복에 둔감해야 함
     (`count(DISTINCT x)`는 둔감, `count(x)`는 민감).
3. `prefer_reachability(max)` — 플래너 통계로 계산한 손익분기점을 넘음
   ([compile.rs:34](../../engine/src/cypher/compile.rs#L34)):
   - `pg_class.reltuples`로 평균 차수 추정. 통계가 없으면 **깊이 ≥ 4**만으로 판정.
   - 예상 walk 수가 `512`를 넘으면 `og_reach`.

**결과**: `MATCH (a)-[:K*1..6]->(b) RETURN count(DISTINCT b)`는 도달성으로,
`MATCH p = (a)-[:K*1..3]->(b) RETURN p`는 트레일 열거로 간다.

전환이 일어나면 컴파일러가 노트를 남긴다:
`variable-length hop compiled as reachability (og_reach): no path is observable, so trails are not enumerated`
([compile.rs:867](../../engine/src/cypher/compile.rs#L867)).

측정 수치와 배경은 [docs/deep-traversal.md](../../docs/deep-traversal.md) 참조.

---

## 6. 결정(Decision) — 주입이 구조적으로 불가능한 이유

- 파라미터는 **jsonb 파라미터 하나**(`$1`)로 바인딩된다
  ([engine/src/cypher/mod.rs:145](../../engine/src/cypher/mod.rs#L145) `exec_json`).
- 값이 SQL 텍스트가 되는 경로가 없으므로, 파라미터가 질의의 **구조**를 바꿀 수 없다.
- 식별자(라벨/프로퍼티 이름)는 `quote_ident`로, 상수 문자열은 `sql_str`로
  이스케이프된다([compile.rs:1584](../../engine/src/cypher/compile.rs#L1584),
  [:1586](../../engine/src/cypher/compile.rs#L1586)).

Bolt 경로도 동일하다 — 드라이버 파라미터는 jsonb로 넘어간다
([bolt/src/session.rs:546](../../bolt/src/session.rs#L546)).

> **예외**: 이 보장은 `og_cypher`에만 적용된다. `og_vector_search(filter)`,
> `og_enable_rls(policy_expr)`, `og_map_table(source_table)`는 **의도적으로**
> SQL 조각을 그대로 받는다 → [06_vector_search.md](06_vector_search.md),
> [08_interop_and_rdf.md](08_interop_and_rdf.md).

---

## 7. 금지 / 필수

- **금지**: Cypher 질의에 `UNION`을 쓰지 말 것. 오류 없이 결과가 잘린다(§4.1).
- **금지**: 결과 jsonb 객체의 키 순서를 컬럼 순서로 믿지 말 것.
  `og_cypher_columns()`를 쓸 것.
- **금지**: 사용자 입력을 질의 텍스트에 문자열 연결하지 말 것. `params`를 쓸 것.
- **필수**: `og_cypher_stats()`는 같은 커넥션에서, 다음 `og_cypher()` 이전에 부를 것.
- **필수**: `og_cypher_explain(..., analyze => true)`가 질의를 **실행**한다는 점을
  기억할 것. 비용을 먼저 보려면 `og_estimate()`.
- **필수**: `og_cypher_check()`의 `ok: true`를 "실행 가능"으로 읽지 말 것 — 파싱만 확인한다.

---

## 8. 관련 문서

- Neo4j 프로시저·인덱스 DDL·Bolt → [09_neo4j_compat.md](09_neo4j_compat.md)
- 순회 함수 상세 → [05_traversal_and_stats.md](05_traversal_and_stats.md)
- 오류 코드 체계 → [11_errors.md](11_errors.md)
- 원문 지원 표 → [docs/cypher.md](../../docs/cypher.md)

<!-- affects: api, backend -->
<!-- requires-update: 02_api/09_neo4j_compat.md, 02_api/11_errors.md, 02_api/12_improvements_api.md -->
