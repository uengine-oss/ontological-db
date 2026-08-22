# Cypher 컴파일러 — `compile.rs` 내부 (1,591줄)

> **이 문서가 답하는 질문**
> - `compile.rs`의 함수는 몇 개이고 각각 무슨 일을 하는가? (함수 단위 지도)
> - 패턴은 어떻게 조인 트리가 되는가?
> - 라벨은 언제 해석되고, 왜 런타임 비용이 0인가?
> - 프로퍼티는 실컬럼과 `__ext` 중 어디로 가는가?
> - 파라미터는 어떻게 바인딩되고, 주입은 어떻게 막히는가?
> - 가변 길이 경로는 언제 `og_vlp`이고 언제 `og_reach`인가?
> - "손익분기 깊이"는 정확히 어느 줄에서 어떻게 계산되는가?

---

## 0. 이 파일의 한 문장 요약

`compile.rs:3-7`:

> The output is ordinary SQL over ordinary relations. That is the whole point:
> PostgreSQL's cost-based optimiser gets to choose the join order, the scan methods
> and the parallelism for the graph pattern, using real statistics on real tables.
> Apache AGE hides the pattern inside an opaque function pipeline and forfeits all of that.

---

## 1. 사실 — 함수 단위 지도

### 1.1 자유 함수 / 타입

| 라인 | 이름 | 역할 |
|---|---|---|
| 18 | `const PARAM: &str = "$1"` | 사용자 `$params`가 담긴 **유일한** 바인딩 파라미터 |
| 34–78 | `fn prefer_reachability(max: u32) -> bool` | ★ 손익분기 계산. 트레일 열거 대신 도달성으로 바꿀지 결정 |
| 82–100 | `fn blind_expr(e: &Expr) -> bool` | 행 중복에 값이 흔들리지 않는 표현식인가 |
| 102–108 | `enum Bind` | `Node` / `Rel` / `Path` / `Scalar` — 변수 바인딩의 4형태 |
| 110–128 | `struct Compiler` | 컴파일 상태 전부 |
| 137–142 | `struct OptionalScope` | 진행 중인 OPTIONAL MATCH의 조인/술어 |
| 144–147 | `struct Compiled` | `{ sql, columns }` |
| 149 | `type CResult<T> = Result<T, String>` | 오류 타입 (문자열) |
| 1575–1580 | `fn mentions_alias(sql, alias) -> bool` | 술어가 이 별칭을 언급하는가 (문자열 검사) |
| 1584–1586 | `pub fn quote_ident(s) -> String` | `"` → `""`, 항상 인용 |
| 1589–1591 | `pub fn sql_str(s) -> String` | `'` → `''` |

### 1.2 `impl Compiler` — 절 / 구조 조립

| 라인 | 함수 | 역할 |
|---|---|---|
| 152–166 | `new(graph)` | `types::graph_id(graph)` 조회 포함 |
| 172–196 | `compile_match(patterns, optional, where_)` | 한 MATCH 절 + 그 WHERE |
| 203–211 | `compile_unwind(expr, alias)` | `LATERAL jsonb_array_elements(...)` 추가 |
| 215–220 | `constrain(sql)` | 술어를 `ON`(optional 중) 또는 `WHERE`로 보냄 |
| 224–237 | `move_join_to_end(idx)` | FROM 항목 하나를 끝으로 이동, 인덱스 재조정 |
| 241–246 | `note_optional_join(alias)` | OPTIONAL 절이 추가한 조인 기록 |
| 252–275 | `close_optional()` | 수집한 술어를 **마지막으로 언급된 별칭의 조인 ON**에 배치 |
| 284–286 | `begin_match_clause()` | `rel_ids` 초기화 (관계 동형성 스코프) |
| 288–291 | `fresh(prefix)` | `n1`, `adj3`, `u4` … 별칭 생성 |
| 293–295 | `binding(v)` | 바인딩 조회 (쓰기 경로용 공개 API) |
| 297–299 | `push_where(sql)` | 무조건 `WHERE`로 (MERGE 고정점 핀 고정용) |
| 302–306 | `bound_vars()` | 정렬된 변수 목록 |
| 310–312 | `build_select_pub(proj)` | `build_select`의 공개 래퍼 |

### 1.3 `impl Compiler` — 읽기 질의

| 라인 | 함수 | 역할 |
|---|---|---|
| 339–349 | `multiplicity_blind(q) -> bool` | ★ 질의가 **경로 개수**를 관측할 수 있는가 |
| 351–370 | `compile_read(q)` | 절 루프 → `build_select` |
| 382–408 | `compile_with(proj, where_)` | 지금까지를 서브쿼리로 봉인, 바인딩 전면 교체 |
| 415–470 | `compile_call(name, args, yields)` | 프로시저를 FROM의 릴레이션으로 |
| 477–600 | `build_core(proj)` | ★ SELECT 리스트 / FROM / WHERE / GROUP BY / ORDER BY / LIMIT / OFFSET / CTE 조립 |
| 602–616 | `build_select(proj)` | `build_core` + `jsonb_build_object(...) AS row` |
| 621–641 | `build_tabular(proj)` | `build_core` + `to_jsonb(cN) AS "name"` (WITH 전용) |

### 1.4 `impl Compiler` — 패턴 → 조인

| 라인 | 함수 | 역할 |
|---|---|---|
| 647–696 | `compile_pattern(p, optional)` | 요소 순회, 노드/홉 교대 처리, 경로 변수 바인딩 |
| 701–707 | `resolve_label_match(labels)` | `(type_id, 매치가능여부)` |
| 709–715 | `resolve_label(labels)` | 매치 불가면 `false` 술어를 밀어넣음 |
| 717–801 | `bind_node(np, optional)` | ★ 노드 바인딩 3분기 (스칼라 승격 / 기존 변수 재사용 / 신규 조인) |
| 803–815 | `push_prop_filters(alias, tid, props)` | 인라인 프로퍼티 → `=` 술어 |
| 819–955 | `join_rel(from, rel, to, optional)` | ★ 관계 조인. 가변 길이 / 단일 홉 분기 |
| 957–969 | `push_rel_prop_filters(...)` | `push_prop_filters`와 **본문이 동일** (`CODE-03`) |

### 1.5 `impl Compiler` — 표현식

| 라인 | 함수 | 역할 |
|---|---|---|
| 976–993 | `prop_sql(alias, tid, prop)` | `(SQL, 알려진 SQL 타입)` |
| 1002–1014 | `prop_sql_json(alias, tid, prop)` | JSON 타입을 보존하는 프로퍼티 접근 (`->`) |
| 1020–1038 | `expr_for_output(e)` | 결과에 실릴 표현식 (프로퍼티만 다름) |
| 1046–1054 | `element_id_sql(v, fname)` | `id()` / `elementId()`의 대상 식별자 |
| 1057–1067 | `type_id_sql(v, fname)` | `labels()` / `type()`의 대상 타입 id |
| 1071–1080 | `restore_bind(var, shadowed)` | 컴프리헨션 바인더 스코프 복원 |
| 1082–1093 | `var_value(v)` | 변수 하나의 전체 값 (노드/엣지 → jsonb) |
| 1095–1113 | `node_json(alias, tid)` | 실컬럼 + `__ext`를 합쳐 노드 jsonb 생성 |
| 1115–1130 | `rel_json(alias, tid)` | 위와 동일 + `_src` / `_dst` |
| 1138–1144 | `jsonb_arg(e)` | 리스트/맵 원소를 `to_jsonb()`가 받을 수 있게 타입 붙임 |
| 1146–1334 | `expr(e, hint)` | ★ 표현식 컴파일 본체 |
| 1336–1393 | `binary(op, l, r)` | 이항 연산 + **타입 지향 강제 변환** |
| 1397–1413 | `type_of(e)` | 알 수 있으면 SQL 타입 |
| 1415–1568 | `func(name, args, distinct)` | 내장 함수 매핑 |

---

## 2. 사실 — 컴파일러 상태

`compile.rs:110-128`:

```rust
pub struct Compiler {
    pub graph: String,
    pub gid: i32,
    binds: HashMap<String, Bind>,   // 변수 → 바인딩
    from: Vec<String>,              // FROM 항목들 (문자열)
    wheres: Vec<String>,            // WHERE 술어들 (문자열)
    ctes: Vec<String>,              // WITH RECURSIVE 항목 (현재 비어 있음)
    n: usize,                       // 별칭 카운터
    rel_ids: Vec<String>,           // 이 MATCH 절에서 지금까지 조인한 홉의 엣지 id 식
    pub notes: Vec<String>,         // 진단용 메모 (spec 008)
    optional: Option<OptionalScope>,
    reachability_only: bool,        // 경로 개수를 아무도 못 보는가
}
```

`ctes`는 `build_core:593-597`에서 `WITH RECURSIVE`로 붙지만, 현재 코드에서
`self.ctes.push(...)`를 하는 곳이 없다 — **죽은 경로**다 (`CODE-14`).

---

## 3. 사실 — 패턴이 조인 트리가 되는 과정

### 3.1 `compile_pattern` (`compile.rs:647-696`)

```
elems = [Node(a), Rel(r), Node(b), Rel(r2), Node(c)]

for elem in elems:
    Node(np) →
        mark = from.len()
        alias = bind_node(np, optional)        # FROM에 릴레이션 추가 (또는 재사용)
        if (prev_node, pending_rel) 둘 다 있으면:
            hop = join_rel(prev, rel, alias, optional)   # FROM에 LATERAL 추가
            hop_exprs.push(hop)
            if 노드 조인이 실제로 추가됐고 && mark > 0 && from.len() > mark+1:
                move_join_to_end(mark)          # 노드 조인을 홉 조인 뒤로 이동
        prev_node = alias
    Rel(rp) → pending_rel = rp
```

`move_join_to_end`(`compile.rs:676-678`)의 이유는 주석에 있다:

> The node is constrained by the hop that reaches it, so its join has to come
> *after* the hop's — otherwise the predicate cannot be an ON condition and the
> node joins to everything. Only matters under OPTIONAL MATCH, where predicates
> live in ON rather than WHERE.

**주의**: 주석은 "OPTIONAL MATCH에서만 중요하다"고 말하지만, 조건에 `optional` 검사가 없다.
즉 일반 MATCH에서도 노드 조인이 홉 뒤로 이동한다. `CROSS JOIN`끼리는 순서가 의미를 갖지 않으므로
의미론적으로는 무해하지만, **생성 SQL의 FROM 순서가 README 예제와 달라질 수 있다** (5절 참조).

### 3.2 경로 변수

`p = (a)-[r]->(b)-[r2]->(c)`이면 `hop_exprs`가 홉별 jsonb 식을 모아
`jsonb_build_array(hop1, hop2)`로 묶여 `Bind::Path`가 된다 (`compile.rs:686-693`).

경로 변수가 있으면 **그 패턴 동안 `reachability_only`가 강제로 꺼진다**
(`compile.rs:657-660`, 복구는 `694`). 경로를 바인딩한다는 건 경로를 관측한다는 뜻이므로.

### 3.3 관계 동형성 (isomorphism)

Cypher는 **한 MATCH 절 안에서 같은 관계를 두 번 지나가지 않는다.**
그래서 `(a)-[:ACTED_IN]->(m)<-[:ACTED_IN]-(b)`가 `a = b`를 내놓지 않는다.

구현: `rel_ids`에 지금까지의 홉 엣지 id 식을 쌓고, 새 홉마다 `<>` 술어를 전부 추가한다
(`compile.rs:908-914`):

```rust
if !optional {
    for other in std::mem::take(&mut self.rel_ids) {
        self.constrain(format!("{u}.eid <> {other}"));
        self.rel_ids.push(other);
    }
    self.rel_ids.push(format!("{u}.eid"));
}
```

스코프 리셋은 `begin_match_clause()`(`compile.rs:284-286`)이고,
`compile_with`도 이를 초기화한다 (`compile.rs:392`).

가변 길이 홉은 이 목록에 들어가지 않는다 — `og_vlp`가 이미 트레일(엣지 비반복) 의미론을 지킨다
(`access.sql:131-132`).

> **주의**: `optional == true`인 홉은 `rel_ids`에 기록되지도, 검사되지도 않는다.
> OPTIONAL MATCH 안의 두 홉은 같은 엣지를 재사용할 수 있다.

---

## 4. 사실 — 라벨 해석 (컴파일 시점, 런타임 비용 0)

### 4.1 흐름

```
bind_node(np)                            compile.rs:717
 └ resolve_label_match(&np.labels)       compile.rs:701
    └ types::resolve_label_set(gid, ..)  catalog/types.rs:152
       └ 각 라벨 → try_type_id           catalog/types.rs:121
       └ 가장 구체적인 것 = 나머지 전부의 서브타입인 것
          └ labeling::og_is_subtype()    catalog/labeling.rs:233  (구간 비교 1회)
 └ LabelMatch::Type(t) → views::ensure_view(t, false)   cypher/views.rs:93
```

`LabelMatch` 3상태 (`catalog/types.rs:192-200`):

| 값 | 의미 | 컴파일러 반응 |
|---|---|---|
| `Any` | 라벨 없음 | `og_data.og_node` 스캔 (`compile.rs:774`) |
| `Type(t)` | 구체 타입 하나 | `og_data.v_<t>` 뷰 스캔 (`compile.rs:773`) |
| `Nothing` | 아무것도 만족 못 함 | `false` 술어 또는 LEFT JOIN `ON false` (`compile.rs:779-792`) |

`Nothing`은 **오류가 아니다.** 존재하지 않는 라벨은 Cypher에서 그냥 아무것도 매치하지 않는다.
철자 힌트는 `notice`로만 나간다 (`catalog/types.rs:161-176`).

### 4.2 타입 뷰

`cypher/views.rs:1-17`가 만드는 것:

```sql
CREATE VIEW og_data.v_1 AS
  SELECT id, 1::int4 AS type_id, p_model, p_year, NULL::int4 AS p_range_km, __ext FROM og_data.n_1
  UNION ALL
  SELECT id, 4::int4, p_model, p_year, p_range_km, __ext FROM og_data.n_4
```

- 자손 집합은 **구간 인덱스 범위 스캔 1회**로 온다 (`labeling::og_subtypes`, `catalog/labeling.rs:193-209`).
- 자손이 안 가진 컬럼은 `NULL::<type>`로 채운다 (`views.rs:110-116`).
- 구체 자손이 없는 추상 타입은 `SELECT ... WHERE false` 형태의 빈 릴레이션 (`views.rs:121-133`).
- 존재 검사가 신선도 검사다 — 스키마가 바뀌면 뷰를 **전부** 지운다 (`views.rs:91-97`, `labeling.rs:175`).

**핵심**: 질의가 실행될 때쯤이면 라벨 술어는 이미 **플래너가 개별적으로 비용을 매길 수 있는
실제 테이블 목록**으로 변해 있다. 행마다 계층을 걷는 일은 없다.

### 4.3 이미 바인딩된 변수의 라벨

`bind_node`(`compile.rs:754-768`) — 같은 변수를 다시 쓸 때는 조인을 추가하지 않고
컴파일 시점에 서브타입 판정만 한다. 모순이면 `false`.

### 4.4 스칼라에서 노드로 승격

`bind_node`(`compile.rs:721-751`) — `CALL … YIELD node`나 `UNWIND nodes AS n`으로
jsonb를 들고 있는 변수도 패턴을 앵커할 수 있다:

```sql
CROSS JOIN og_data.og_node n5
-- WHERE n5.id = ((<scalar sql>) ->> '_id')::int8
```

---

## 5. 사실 — 컴파일 산출물 (실제 예제)

`README.md:230-247`이 문서화한 산출물:

```sql
SELECT og_cypher_sql('default',
  $$ MATCH (p:Person)-[:ACTED_IN]->(w:Work) WHERE p.born > 1960 RETURN w.title $$);
```

```sql
SELECT jsonb_build_object('w.title', t.c0) AS row FROM (
SELECT n2.p_title AS c0 FROM og_data.v_5 n1
  CROSS JOIN og_data.v_2 n2
  CROSS JOIN LATERAL (SELECT u.nbr, u.eid FROM og_data.og_adj adj3,
                      LATERAL unnest(adj3.nbr, adj3.eid) AS u(nbr, eid)
                      WHERE adj3.src = n1.id AND adj3.dir = 'o'::"char"
                        AND adj3.etype = ANY(ARRAY[7]::int4[])) u4
 WHERE n2.id = u4.nbr AND (n1.p_born > 1960)
) t
```

### 5.1 각 조각의 생성 지점

| 조각 | 생성 코드 |
|---|---|
| `SELECT jsonb_build_object('w.title', t.c0) AS row FROM (...) t` | `build_select` `compile.rs:609-615` |
| 키 `'w.title'` | `Expr::default_alias()` `ast.rs:244-247` → `sql_str` `compile.rs:607` |
| `n2.p_title AS c0` | `build_core` `compile.rs:493-498`, 프로퍼티는 `prop_sql_json` `compile.rs:1002` |
| `og_data.v_5 n1` | `bind_node` `compile.rs:771-784` + `views::ensure_view` `views.rs:93` |
| `CROSS JOIN og_data.v_2 n2` | `bind_node` `compile.rs:791` |
| `CROSS JOIN LATERAL (…) u4` | `join_rel` `compile.rs:888-904` |
| `adj3.dir = 'o'::"char"` | `join_rel` `compile.rs:896-899` (`Dir::Out` → `'o'`) |
| `adj3.etype = ANY(ARRAY[7]::int4[])` | `join_rel` `compile.rs:833-849` — `og_subtypes`로 확장된 타입 id 목록 |
| `WHERE n2.id = u4.nbr` | `join_rel` `compile.rs:906` |
| `AND (n1.p_born > 1960)` | `binary` `compile.rs:1381`, `compile_match` `compile.rs:186-189` |
| 별칭 `n1 n2 adj3 u4` | `fresh()` `compile.rs:288-291` (호출 순서대로 1,2,3,4) |

### 5.2 README 예제와 현재 코드의 차이 (주의)

`compile_pattern`의 `move_join_to_end`(`compile.rs:676-678`)는 `optional` 여부를 검사하지 않으므로,
**현재 코드는 `CROSS JOIN og_data.v_2 n2`를 LATERAL 뒤로 옮길 것으로 읽힌다.**
즉 실제 출력의 FROM 항목 순서가 위 예제와 다를 수 있다.

정확한 현재 출력은 **직접 확인해야 한다**:

```sql
SELECT og_cypher_sql('default',
  $$ MATCH (p:Person)-[:ACTED_IN]->(w:Work) WHERE p.born > 1960 RETURN w.title $$);
```

이 문서는 두 순서 모두 의미론적으로 동일하다는 사실(모두 `CROSS JOIN`이고 상관 참조는
`n1`만 향한다)까지만 단언하며, **어느 순서로 출력되는지는 미확인**이다. → `CODE-15`.

### 5.3 프로퍼티 접근이 컴파일되는 3가지 형태

`prop_sql` (`compile.rs:976-993`) / `prop_sql_json` (`compile.rs:1002-1014`):

| 상황 | `prop_sql` (텍스트) | `prop_sql_json` (JSON 타입 보존) |
|---|---|---|
| `id` / `_id` | `n1.id` (타입 `int8`) | `to_jsonb(n1.id)` |
| 타입 알려짐 + 선언된 프로퍼티 | `n1.p_born` (타입 = 선언 타입) | `to_jsonb(n1.p_born)` |
| 타입 알려짐 + 미선언 | `(n1.__ext->>'x')` (타입 미상) | `(n1.__ext -> 'x')` |
| 타입 미상 | `(og_node_json(n1.id)->>'x')` | `(og_node_json(n1.id) -> 'x')` |

`prop_sql_json`이 따로 있는 이유 (`compile.rs:995-1001`):
`->>`는 text를 내므로 `20`이 `"20"`으로 나간다. Neo4j는 정수를 준다.
그래서 **결과에 실릴 때만** `->`를 쓴다.

**마지막 행이 성능 함정이다**: `og_node_json`은 `LANGUAGE plpgsql`이라 인라인되지 않는다
(`access.sql:209`). 라벨 없는 패턴 `MATCH (n) WHERE n.x = 1`은 행마다 plpgsql 호출을 한다.

---

## 6. 사실 — 파라미터 바인딩과 주입 방지

`compile.rs:18` — 사용자 파라미터가 들어올 수 있는 자리는 **`$1` 하나**뿐이다.

`compile.rs:1156-1162`:

```rust
Expr::Param(p) => {
    let base = format!("({PARAM} ->> {})", sql_str(p));
    match hint {
        Some(t) if t != "text" => format!("{base}::{t}"),
        _ => base,
    }
}
```

- `sql_str(p)`가 파라미터 **이름**을 이스케이프한다 (`'` → `''`).
- 파라미터 **값**은 SQL 텍스트에 들어가지 않는다. 실행 시 jsonb 하나로 바인딩된다:
  `client.select(sql, None, &[JsonB(params.clone()).into()])` (`cypher/mod.rs:148`).

### 6.1 타입 지향 강제 변환 — 인덱스를 살리기 위한 것

`binary` (`compile.rs:1336-1368`):

```rust
let lhint = self.type_of(r);   // 왼쪽은 오른쪽의 타입을 힌트로 받는다
let rhint = self.type_of(l);   // 그 반대도
let mut ls = self.expr(l, lhint.as_deref())?;
let mut rs = self.expr(r, rhint.as_deref())?;
```

`p.born > $year`에서 `p.born`이 `int8` 컬럼이면 `$year`는
`($1 ->> 'year')::int8`로 컴파일된다. **텍스트 비교가 아니므로 그 컬럼의 인덱스가 그대로 쓰인다.**

미선언 프로퍼티(타입 미상)가 숫자와 비교되면, PostgreSQL이 `text = integer`를 거절하므로
미상 쪽에 캐스트를 붙인다 (`compile.rs:1359-1368`).

### 6.2 연산자 매핑에서 눈에 띄는 것들

`compile.rs:1370-1392`:

| Cypher | SQL | 이유 |
|---|---|---|
| `=` | `IS NOT DISTINCT FROM` | Cypher의 `=`는 NULL에서도 정의된다 |
| `<>` | `IS DISTINCT FROM` | 같은 이유 |
| `/` | `({ls} / NULLIF({rs}, 0))` | 0 나눗셈이 오류가 아니라 NULL |
| `%` | `(({ls})::numeric % NULLIF(({rs})::numeric, 0))` | |
| `x IN xs` | `((xs) @> to_jsonb(x))` | jsonb 포함 연산자 (`compile.rs:1345-1349`) |
| `CONTAINS` | `strpos(a::text, b::text) > 0` | |
| `STARTS WITH` | `a::text LIKE b::text \|\| '%'` | |
| `=~` | `a::text ~ b::text` | POSIX 정규식 |

---

## 7. 사실 — 가변 길이 경로

`join_rel`(`compile.rs:856-886`)의 분기:

```rust
if let Some((min, max)) = rel.range {
    let w = self.fresh("vl");
    let joiner = if optional { "LEFT JOIN LATERAL" } else { "CROSS JOIN LATERAL" };
    let f = if rel.var.is_none() && self.reachability_only && prefer_reachability(max) {
        self.notes.push(...);
        "og_reach"
    } else {
        "og_vlp"
    };
    self.from.push(format!(
        "{joiner} {f}({from_alias}.id, {etype_pred}, {dir_lit}::\"char\", {min}, {max}) {w}{on}"
    ));
    self.note_optional_join(&w);
    self.constrain(format!("{to_alias}.id = {w}.node"));
    if let Some(v) = &rel.var {
        self.binds.insert(v.clone(), Bind::Path { hops_expr: format!("to_jsonb({w}.path)") });
    }
    return Ok(format!("to_jsonb({w}.path)"));
}
```

**두 함수의 시그니처가 `path` 컬럼 전까지 동일하다** — 그래서 같은 LATERAL 자리에
글자만 바꿔 끼울 수 있다 (`storage/traverse.rs:77-79`).

| | `og_vlp` | `og_reach` |
|---|---|---|
| 정의 | `access.sql:138-156` (`LANGUAGE sql`) | `storage/traverse.rs:80-161` (Rust) |
| 반환 | `(node, depth, path int8[])` | `(node, depth)` |
| 의미 | 트레일 열거 — 경로당 1행, `degree^k` | 도달성 — 노드당 1행, `\|V\|+\|E\|` 상한 |
| 인라인 | ✅ (SQL 함수) | ❌ (Rust SRF) |
| 병렬성 | `PARALLEL SAFE` | `parallel_restricted` |
| ROWS | `ROWS 100` (`access.sql:140`) | `ROWS 100` (`access.sql:197`로 맞춤) |

`access.sql:192-197`의 주석이 `ROWS`를 맞춰준 이유를 기록한다:
pgrx는 SRF에 PostgreSQL 기본값 1000을 준다. 같은 질문에 답하는 두 함수의 비용이
10배 차이 나면 플래너가 서로 다른 조인 순서를 고르고, 비교가 "추정치"를 측정하게 된다.

---

## 8. ★ 방문집합 BFS로 전환하는 조건

전환은 **세 조건의 AND**다 (`compile.rs:865`).

```rust
rel.var.is_none() && self.reachability_only && prefer_reachability(max)
```

### 조건 1 — 관계 변수가 없을 것 (`compile.rs:865`, `rel.var.is_none()`)

`-[e:R*1..6]->`처럼 관계 변수를 바인딩하면 다중도를 관측할 수 있다. 무조건 `og_vlp`.

### 조건 2 — 경로 변수가 없을 것 (`compile.rs:657-660`)

```rust
let outer_reachability = self.reachability_only;
if p.path_var.is_some() {
    self.reachability_only = false;
}
```

`MATCH p = (a)-[*1..3]->(b)`는 경로 자체를 바인딩하므로 열거해야 한다.

### 조건 3 — 질의가 다중도에 눈이 멀었을 것 (`multiplicity_blind`, `compile.rs:339-349`)

```rust
fn multiplicity_blind(q: &Query) -> bool {
    if q.clauses.iter().any(|c| matches!(c, Clause::With { .. })) {
        return false;                                     // ① WITH가 있으면 포기
    }
    let Some(Clause::Return(p)) = q.clauses.last() else { return false };
    if p.distinct {
        return true;                                      // ② RETURN DISTINCT면 통과
    }
    let exprs = || p.items.iter().map(|i| &i.expr).chain(p.order.iter().map(|o| &o.expr));
    exprs().any(|e| e.is_aggregate()) && exprs().all(blind_expr)   // ③
}
```

① `WITH`는 RETURN 전에 집계할 수 있고 이 패스는 그 안을 보지 않는다 → 보수적으로 거부.
② `RETURN DISTINCT`는 중복 행이 살아남을 수 없다 → 통과.
③ 그 외에는 **집계가 하나라도 있어야 하고, 모든 항목이 `blind_expr`을 통과해야** 한다.

`blind_expr`(`compile.rs:82-100`):

| 표현식 | 눈먼가? |
|---|---|
| `min(x)` / `max(x)` | ✅ 항상 |
| `count(DISTINCT x)` / `collect(DISTINCT x)` | ✅ (`*` 인자는 제외) |
| `count(x)` / `count(*)` / `collect(x)` | ❌ |
| `sum` / `avg` / `stdev` / 사용자 정의 | ❌ |
| 비집계 표현식 | ✅ (재귀적으로 검사) |

`compile.rs:327-328`이 이 좁은 판정의 이유를 명시한다:

> The test is deliberately narrow, because being wrong here changes answers
> rather than timings.

**검증**: `engine/tests/sql/05_reachability.sql:72-91`이 6개 케이스를 고정한다.

| 질의 | 기대 |
|---|---|
| `RETURN count(DISTINCT y)` | `og_reach` |
| `RETURN DISTINCT y.name` | `og_reach` |
| `RETURN count(y)` | `og_vlp` |
| `RETURN y.name` | `og_vlp` |
| `MATCH p = … RETURN DISTINCT p` | `og_vlp` |
| `MATCH …-[e:E*1..12]->… RETURN count(DISTINCT y)` | `og_vlp` |

---

## 9. ★ 손익분기 깊이를 플래너 통계로 계산하는 부분

**`compile.rs:34-78`, `fn prefer_reachability(max: u32) -> bool`. 전문:**

```rust
fn prefer_reachability(max: u32) -> bool {
    const WALKS: f64 = 512.0;   // line 42
    const DEEP: u32 = 4;        // line 44

    let est = crate::spiu::two::<f32, f32>(                       // line 46
        "SELECT (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_node'::regclass),
                (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_edge'::regclass)",
        &[],
    );
    let (nodes, edges) = match est {                              // line 51
        Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => (n as f64, e as f64),
        _ => return max >= DEEP,                                  // line 53
    };
    let degree = (edges / nodes).max(1.0);                        // line 58

    let mut walks = 0.0f64;                                       // line 68
    let mut level = 1.0f64;
    for _ in 0..max {                                             // line 70
        level *= degree;                                          // line 71
        walks += level;                                           // line 72
        if walks > WALKS || !walks.is_finite() {                  // line 73
            return true;                                          // line 74
        }
    }
    false                                                         // line 77
}
```

### 9.1 통계 출처 — `compile.rs:46-50`

`pg_class.reltuples` 두 개다. **카탈로그 조회이지 스캔이 아니다.**
`compile.rs:30-31`:

> Both terms come from the planner's own statistics — a catalog lookup, not a scan —
> so this costs nothing to ask.

### 9.2 통계가 없을 때 — `compile.rs:51-54`

`reltuples`는 `ANALYZE` 전에는 `-1` 또는 `0`이다. 그러면 `_ => return max >= DEEP`,
즉 **깊이 4 이상이면 무조건 전환**한다.

`engine/tests/sql/05_reachability.sql:73-77`이 이 폴백을 명시적으로 이용한다 —
`ANALYZE` 전에 12홉으로 테스트해서 **의미론 조건만** 격리한다.

### 9.3 평균 out-degree — `compile.rs:58`

```rust
let degree = (edges / nodes).max(1.0);
```

관계 타입별 차수가 더 정확하겠지만, `compile.rs:55-57`:

> A per-relation-type figure would be sharper, but this decision only has to be
> right about an order of magnitude, and it must not cost a scan to make.

### 9.4 손익분기 조건 — `compile.rs:68-76`

`Σ(i=1..max) degree^i > 512`.

`WALKS = 512`는 유도값이 아니라 **측정으로 맞춘 값**이며, 의도적으로 낮다 (`compile.rs:36-41`):

> The two failure modes are not symmetric: enumerating when we should not have
> runs out of time or memory — 2.7 s at twenty hops on a lattice, 90 s at thirty —
> while reaching when we should not have costs a bounded fraction of a millisecond.

### 9.5 폐기된 이전 규칙 — `compile.rs:60-67`

이전 버전은 `Σ degree^i`를 `|V|`와 비교했다. 실패 사례가 주석에 남아 있다:
1000×1000 격자, 10홉 → 2,046 walks vs 100만 노드 → "감당 가능"으로 판정되지만
실제 도달 노드는 66개뿐이었고, 열거 3.83 ms vs 도달성 0.30 ms.
그래서 규칙이 **"올 walks가 전환 비용을 갚을 만큼 많은가"** 하나로 축소됐다.

### 9.6 검증

`engine/tests/sql/05_reachability.sql:93-96`:

```sql
ANALYZE;
SELECT og_cypher_sql('r', $$MATCH (x:N {name:'a'})-[:E*1..2]->(y:N) RETURN count(DISTINCT y)$$)
       LIKE '%og_vlp(%' AS shallow_keeps_vlp;
```

4노드/5엣지 그래프에서 degree ≈ 1.25, 2홉이면 walks ≈ 2.8 < 512 → `og_vlp` 유지.

`engine/tests/sql/05_reachability.sql:98-101`은 세 경로가 **같은 답**을 내는지도 확인한다.

### 9.7 비용

`prefer_reachability`는 **가변 길이 홉마다 한 번** SPI 질의를 낸다.
컴파일 결과는 `PLAN_CACHE`(`cypher/mod.rs:26-31`)에 캐시되므로 같은 질의 문자열에 대해
반복되지는 않지만, 캐시 미스마다 발생한다. → `CODE-10`.

---

## 10. 사실 — OPTIONAL MATCH가 LEFT JOIN이 되는 방식

문제 (`compile.rs:130-136`):

> An optional pattern's predicates cannot go in `WHERE`: `A LEFT JOIN B ON true
> WHERE b.x = a.y` throws away exactly the rows the LEFT JOIN was there to keep.

해법:

1. `compile_match`가 `OptionalScope`를 연다 (`compile.rs:179-181`).
2. `constrain()`이 `WHERE` 대신 `scope.preds`에 쌓는다 (`compile.rs:215-220`).
3. 조인이 추가될 때마다 `note_optional_join(alias)`가 (별칭, FROM 인덱스)를 기록한다.
4. `close_optional()`(`compile.rs:252-275`)이 술어마다
   **그 술어가 언급하는 별칭 중 FROM 인덱스가 가장 큰 것**의 조인 `ON`에 붙인다.
   아무 별칭도 언급하지 않으면 바깥 `WHERE`로 간다.

별칭 언급 판정은 `mentions_alias`(`compile.rs:1575-1580`) — `"{alias}."` 부분 문자열을 찾되
앞 글자가 영숫자/`_`가 아닌 경우만. `n1`이 `n10` 안에서 매치되지 않게 하려는 것이다.

> **한계**: 이것은 문자열 검사이므로 SQL 문자열 리터럴 안의 `'n1.x'`도 매치한다. → `CODE-04`.

---

## 11. 사실 — `WITH`는 지평선이다

`compile_with`(`compile.rs:382-408`):

```rust
let inner = self.build_tabular(proj)?;
let alias = self.fresh("w");
self.from  = vec![format!("({}) AS {alias}({})", inner.sql, quoted.join(", "))];
self.wheres.clear();
self.rel_ids.clear();
self.binds = /* 투영된 컬럼만 Bind::Scalar 로 */;
```

- 지금까지의 전부가 서브쿼리 하나가 된다.
- **투영된 이름만 살아남는다** — Cypher의 "WITH is the horizon" 규칙이 강제가 아니라
  구현에서 떨어져 나온다 (`compile.rs:372-378`).
- `WITH … WHERE`는 새 바인딩에 대해 컴파일되므로 자연히 `HAVING`이 된다 (`compile.rs:401-406`).
- `build_tabular`(`compile.rs:621-641`)는 **모든 컬럼을 jsonb로** 넘긴다. 이유는 `compile.rs:623-628`:
  그러지 않으면 다음 세그먼트가 "이 바인딩이 노드(jsonb)인가 카운트(bigint)인가"를
  알아야 비교를 컴파일할 수 있다.

---

## 12. 사실 — `build_core`가 조립하는 것

`compile.rs:477-600`. 반환값은 `(with, inner, tail, names)` 4-튜플.

| 산출 | 라인 | 비고 |
|---|---|---|
| `RETURN *` 확장 | 484–492 | `binds` 키 전부. **HashMap 순서라서 비결정적** → `CODE-16` |
| `c0, c1, …` 컬럼 별칭 | 498 | |
| 자동 `GROUP BY` | 495–497, 544–546 | 집계가 하나라도 있으면 비집계 항목 전부를 그룹 키로 |
| `ORDER BY` 별칭 해석 | 502–549 | RETURN 별칭을 다시 참조 가능. 집계 별칭은 GROUP BY에 넣지 않음 (517–528) |
| `o0, o1, …` 정렬 컬럼 | 547–548 | 바깥 쿼리에서 `t.o0`으로 참조 |
| `FROM` / `WHERE` / `GROUP BY` | 551–565 | `wheres`는 `\n   AND `로 연결 |
| `DISTINCT` | 573 | |
| `LIMIT` / `OFFSET` | 584–591 | `int8` 힌트로 컴파일 |
| `WITH RECURSIVE` | 593–597 | `ctes`가 비어 있으므로 현재 항상 빈 문자열 |

---

## 13. 사실 — `CALL … YIELD`

`compile_call`(`compile.rs:415-470`):

1. 인자를 `procs::Arg`로 분류한다 (`compile.rs:424-435`):
   - 문자열 리터럴 → `Arg::Str` (인덱스 이름은 컴파일 시점에 읽혀야 한다)
   - 바인딩된 노드 변수 → `Arg::NodeId("n1.id")` — **jsonb가 아니라 컬럼을 넘겨서 조인을 유지**
   - 그 외 → `Arg::Sql`
2. `procs::plan(...)`이 FROM 릴레이션과 컬럼 목록을 돌려준다 (`compat/procs.rs:80-161`).
3. `plan.lateral`이면 `CROSS JOIN LATERAL`, 아니면 `CROSS JOIN` (`compile.rs:440-447`).
4. `YIELD`가 없으면 모든 컬럼이 자기 이름으로 스코프에 들어온다 (`compile.rs:451-455`).

레지스트리는 **닫혀 있다** — 모르는 프로시저는 지원 목록과 함께 거절된다
(`compat/procs.rs:154-159`).

---

## 14. 사실 — 함수 매핑 (`func`, `compile.rs:1415-1568`)

집계 (`compile.rs:1420-1441`): `count` `sum` `avg` `min` `max` `collect`(→`jsonb_agg`) `stdev`(→`stddev_samp`).

그래프 함수:

| Cypher | SQL |
|---|---|
| `id(x)` | `element_id_sql` — 바인딩 종류별로 다름 (`compile.rs:1046`) |
| `elementId(x)` | 위 결과에 `::text` |
| `type(r)` | `og_type_name(<type_id 식>)` |
| `labels(n)` | `to_jsonb(ARRAY(SELECT og_type_name(t) FROM unnest(og_supertypes(<tid>)) AS t))` |

`labels()`가 **리스트**를 돌려주는 이유 (`compile.rs:1465-1469`): 여기서는 노드가
상위 타입 사슬의 모든 이름을 실제로 갖고 있다. 그래야 `'Foo' IN labels(n)`이
`(:Super:Sub)` 그래프에서 Neo4j와 같은 뜻이 된다.

문자열/수치/변환: `toUpper toLower trim substring replace split coalesce abs ceil floor round sqrt
rand toString toInteger toFloat timestamp datetime exists keys length size`.

`coalesce`(`compile.rs:1503-1520`)만 특수하다 — SQL은 모든 분기가 한 타입이어야 하는데
미선언 프로퍼티는 text로 나온다. 그래서 타입을 아는 첫 분기의 타입으로 나머지를 읽는다.

벡터 (`compile.rs:1536-1541`): `vector.similarity` → `1 - (a <=> b)`,
`vector.distance` → `<=>`, `vector.l2` → `<->`.

`genai.vector.encode`(`compile.rs:1545-1557`) → `og_genai_encode(text, provider, config)`.

모르는 함수는 **지원 목록과 함께** 거절된다 (`compile.rs:1558-1566`).

---

## 15. 결정 요약

| 결정 | 근거 | 대가 |
|---|---|---|
| SQL 문자열을 직접 조립 (AST/빌더 없음) | 없음 — 관례 | 문자열 검사 기반 로직(`mentions_alias`)의 취약성 (`CODE-04`) |
| 라벨은 컴파일 시점에 뷰로 해석 | `compile.rs:9-10`, `views.rs:14-17` | 스키마 변경 시 뷰 폐기 + 플랜 캐시 불일치 (`CODE-01`) |
| 파라미터는 `$1` jsonb 하나 | spec 003 FR-026 | 없음 |
| 도달성 재작성은 3중 AND | `compile.rs:327-328` | 보수적 — `WITH`가 있으면 무조건 포기 |
| 손익분기는 `reltuples`만 사용 | `compile.rs:29-33` | 관계 타입별 차수 편차를 못 봄 |
| 오류 타입은 `Result<T, String>` | `compile.rs:149` | 오류 코드/분류 불가, Bolt가 문자열 매칭으로 복원 (`CODE-11`) |

---

## 금지 / 필수

- **금지**: `format!`으로 사용자 **값**을 SQL에 넣는 것. `$1`을 통해서만.
  라벨/프로퍼티 이름 같은 식별자는 `quote_ident`, 문자열 리터럴은 `sql_str`.
- **금지**: `multiplicity_blind` / `blind_expr`의 판정을 넓히는 것.
  여기서 틀리면 **성능이 아니라 답이 바뀐다** (`compile.rs:327-328`).
  넓힐 때는 `engine/tests/sql/05_reachability.sql`에 케이스를 먼저 추가한다.
- **금지**: `og_reach`와 `og_vlp` 중 하나의 시그니처만 바꾸는 것.
  `join_rel:874-876`이 같은 자리에 글자만 바꿔 끼운다.
- **금지**: `og_vlp` / `og_reach`의 `ROWS` 추정치를 한쪽만 바꾸는 것 (`access.sql:192-197`).
- **필수**: `#[pg_extern(stable)]`로 노출되는 함수가 `views::ensure_view`를 부르면
  그것은 DDL이며 STABLE 계약 위반이다 (`cypher/mod.rs:74-80`). → `CODE-02`.
- **필수**: 새 내장 함수를 추가하면 `func`의 오류 메시지 목록(`compile.rs:1560-1564`)도 갱신한다.
  에이전트가 그 목록을 읽고 재시도한다 (spec 008).

<!-- affects: backend, api, performance -->
<!-- requires-update: 02_api/02_cypher_api.md, 08_operations/ -->
