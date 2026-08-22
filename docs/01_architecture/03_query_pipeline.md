# 질의 파이프라인 — Cypher/TypeQL 한 줄이 디스크 읽기가 되기까지

> **이 문서가 답하는 질문**
> - `og_cypher('kb', $$ MATCH … $$)` 를 호출하면 정확히 무슨 일이 순서대로 일어나는가?
> - 읽기 질의와 쓰기 질의는 어디서 갈라지는가?
> - TypeQL 파이프라인은 Cypher와 무엇을 공유하고 무엇을 공유하지 않는가?
> - 어느 단계가 캐시되고, 어느 단계가 매번 실행되는가?

---

## 1. 전체 흐름 (읽기 질의)

```
og_cypher('kb', $$ MATCH (p:Person)-[:ACTED_IN]->(w:Work)
                   WHERE p.born > 1960 RETURN w.title $$)
  │
  ├─[1] stats::reset()                     백엔드 로컬 쓰기 카운터 초기화
  ├─[2] parser::parse(query)               → Query { clauses, union }
  ├─[3] is_write(&ast)?                    Create|Merge|Set|Remove|Delete|Ddl 존재?
  │        └─ no → run_read
  ├─[4] compile_cached(graph, query)
  │       ├─ PLAN_CACHE 조회 — (graph, query) 키
  │       └─ miss:
  │           ├─ Compiler::new(graph)      graph_id 조회 (SPI)
  │           ├─ multiplicity_blind(q)     도달성 재작성 가능 여부 판정
  │           ├─ 절마다 compile_*()
  │           │   ├─ 라벨 → 타입 id  (구간 인덱스, SPI)
  │           │   ├─ ensure_view()   ★ CREATE OR REPLACE VIEW (DDL!)
  │           │   ├─ 관계 타입 → 서브타입 id 배열 리터럴
  │           │   ├─ 프로퍼티 → 실 컬럼명 + SQL 타입
  │           │   └─ 가변 길이 홉 → og_vlp | og_reach 결정
  │           └─ build_select(proj)        → SQL 문자열 + 컬럼 이름
  ├─[5] exec_json(sql, params)             Spi::connect → client.select(sql, None, [$1=jsonb])
  ├─[6] PostgreSQL 플래너/실행기            조인 순서·스캔·병렬성 결정 → 힙/인덱스 읽기
  ├─[7] 각 행의 1번 컬럼(jsonb)을 수집
  └─[8] audit(...)                         og_data.og_audit INSERT
```

근거: [`engine/src/cypher/mod.rs:82-152`](../../engine/src/cypher/mod.rs),
[`engine/src/cypher/compile.rs:351-370`](../../engine/src/cypher/compile.rs).

---

## 2. 단계별 상세

### [2] 파싱 — 손으로 쓴 재귀 하강

`engine/src/cypher/lexer.rs` (302줄) → `parser.rs` (1,177줄) → `ast.rs` (263줄).

- 파서는 **의미 검증을 하지 않는다.** 존재하지 않는 라벨도 파싱은 통과한다.
- 미지원 구문은 명시적 오류를 낸다: `"unexpected clause '{}'"`
  ([`parser.rs:148-150`](../../engine/src/cypher/parser.rs)).
- **예외**: `UNION` 은 파싱되어 `Query.union` 에 저장되지만 이후 아무도 읽지 않는다
  ([`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-01**).

### [3] 읽기/쓰기 분기

```rust
fn is_write(q: &Query) -> bool {
    q.clauses.iter().any(|c| matches!(c,
        Clause::Create(_) | Clause::Merge { .. } | Clause::Set(_)
        | Clause::Remove(_) | Clause::Delete { .. } | Clause::Ddl(_)))
}
```
— [`engine/src/cypher/mod.rs:33-45`](../../engine/src/cypher/mod.rs)

이 판정은 Bolt 게이트웨이가 `og_cypher_check()` 를 통해 재사용한다
([`bolt/src/session.rs:262-266`](../../bolt/src/session.rs)) —
게이트웨이가 Cypher를 다시 읽지 않는 이유다.

### [4] 컴파일 — 이 파이프라인의 핵심

#### 4a. 라벨 해소 (컴파일 타임, 런타임 비용 0)

```
(p:Person)
  → resolve_label_set(gid, "kb", ["Person"])      catalog/types.rs:152
  → LabelMatch::Type(2)
  → views::ensure_view(2, false)                  cypher/views.rs:93
  → "og_data.v_2"
```

`og_data.v_2` 는 `Person` 과 그 모든 후손 타입의 구체 테이블을
`UNION ALL` 한 생성 뷰다. 후손 집합은 **구간 인덱스 범위 스캔 한 번**으로 얻는다.

**라벨 집합의 해소 규칙** (Neo4j와 다른 지점):
Neo4j는 노드가 독립적인 라벨 여러 개를 가질 수 있지만, 여기서는 노드가 타입 하나를 갖고
위쪽 이름들을 상속한다. 그래서 `(:_Entity:Doc)` 은 **가장 구체적인 멤버**로 해소되고,
가장 구체적인 멤버가 없는 집합(서로 무관한 두 라벨)은 **아무것도 매칭하지 않는다**
— 추측하지 않고 그렇게 보고한다 ([`catalog/types.rs:140-200`](../../engine/src/catalog/types.rs)).

#### 4b. 홉 컴파일

**고정 길이 홉** — `og_adj` LATERAL 조인:

```sql
CROSS JOIN LATERAL (SELECT u.nbr, u.eid FROM og_data.og_adj adj3,
                    LATERAL unnest(adj3.nbr, adj3.eid) AS u(nbr, eid)
                    WHERE adj3.src = n1.id AND adj3.dir = 'o'::"char"
                      AND adj3.etype = ANY(ARRAY[7]::int4[])) u4
```
— [`compile.rs:888-906`](../../engine/src/cypher/compile.rs)

`ARRAY[7]` 은 `ACTED_IN` 과 그 모든 서브타입 id다. 컴파일 타임에 확정된다.

**동형 매칭**: 같은 MATCH 절 안의 이전 홉들과 `{u}.eid <> {other}` 로 구별한다
([`compile.rs:908-914`](../../engine/src/cypher/compile.rs)). 절이 바뀌면 리셋된다.

**가변 길이 홉** — 두 갈래:

```rust
let f = if rel.var.is_none() && self.reachability_only && prefer_reachability(max) {
    "og_reach"      // 방문집합 BFS
} else {
    "og_vlp"        // 트레일 열거
};
```
— [`compile.rs:865-873`](../../engine/src/cypher/compile.rs)

세 조건이 **모두** 참일 때만 `og_reach` 다:

| 조건 | 의미 | 근거 |
|---|---|---|
| `rel.var.is_none()` | 관계 변수를 바인딩하지 않음 | 바인딩하면 다중도가 관측된다 |
| `reachability_only` | 질의가 경로 다중도를 볼 수 없음 | [`compile.rs:318-349`](../../engine/src/cypher/compile.rs) |
| `prefer_reachability(max)` | 예상 걸음 수 `Σ degreeⁱ > 512` | [`compile.rs:34-78`](../../engine/src/cypher/compile.rs) |

또한 패턴이 경로 변수(`MATCH p = …`)를 바인딩하면 그 패턴 동안
`reachability_only` 가 강제로 꺼진다 ([`compile.rs:655-660, 694`](../../engine/src/cypher/compile.rs)).

`prefer_reachability` 의 통계는 `pg_class.reltuples` **카탈로그 조회**이지 스캔이 아니다.
통계가 없는 데이터베이스는 깊이만으로 판단한다(`DEEP = 4`).
비대칭 손실이 근거다: 잘못 열거하면 시간/메모리가 터지고(20홉 격자 2.7초, 30홉 90초),
잘못 도달성으로 가면 밀리초의 유한한 일부만 손해다.

#### 4c. 프로퍼티 접근

```rust
pub fn prop_sql(&self, alias, tid, prop) -> (String, Option<String>)
```
— [`compile.rs:976-993`](../../engine/src/cypher/compile.rs)

| 상황 | SQL | 타입 정보 |
|---|---|---|
| `id` / `_id` | `{alias}.id` | `int8` |
| 선언된 프로퍼티 | `{alias}.p_born` | 선언 타입 |
| 미선언 프로퍼티 (타입 뷰) | `({alias}.__ext->>'x')` | 없음 (text) |
| 타입 미상 변수 | `(og_node_json({alias}.id)->>'x')` | 없음 |

**출력용은 다르다**: `prop_sql_json()` 은 `->>` 대신 `->` 를 쓴다.
`->>` 는 text를 내므로 `20` 이 `"20"` 이 되어 Neo4j와 다른 결과가 나온다
([`compile.rs:995-1014`](../../engine/src/cypher/compile.rs)).

#### 4d. 타입 방향 캐스팅

비교의 **양쪽을 서로의 타입 힌트로** 컴파일한다:

```rust
let lhint = self.type_of(r);   // 오른쪽 타입 → 왼쪽 힌트
let rhint = self.type_of(l);   // 왼쪽 타입 → 오른쪽 힌트
```
— [`compile.rs:1351-1354`](../../engine/src/cypher/compile.rs)

효과: `$born` 파라미터가 `p_born int4` 와 비교되면 `int4` 로 캐스팅되어
**인덱스가 살아 있다**. 그리고 사용자 값이 SQL 텍스트에 들어가지 않으므로
주입이 구조적으로 불가능하다.

미선언 프로퍼티(text)를 숫자와 비교할 때는 반대쪽 타입으로 읽어낸다
— `text = integer` 를 PostgreSQL이 거부하기 때문
([`compile.rs:1356-1368`](../../engine/src/cypher/compile.rs)).

#### 4e. `WITH` — 지평선(horizon)

```rust
self.from = vec![format!("({}) AS {alias}({})", inner.sql, quoted.join(", "))];
self.wheres.clear();
self.rel_ids.clear();
self.binds = /* 프로젝션된 이름만 */;
```
— [`compile.rs:382-408`](../../engine/src/cypher/compile.rs)

지금까지 컴파일된 것이 **서브쿼리**가 되고, 프로젝션된 이름만 이후에 보인다.
Cypher의 "WITH 는 지평선" 규칙이 별도 강제 없이 구현에서 떨어져 나온다.
집계는 공짜다 — `WITH a, count(*) AS n` 은 그룹화된 SELECT이고,
그 뒤의 `WHERE` 는 그룹 행을 거르므로 `HAVING` 이다.

**지평선을 넘는 모든 컬럼은 jsonb가 된다** (`to_jsonb(t.c{i})`,
[`compile.rs:629-633`](../../engine/src/cypher/compile.rs)).
대안(각 컬럼의 SQL 타입 유지)은 다음 세그먼트가 어떤 바인딩이 노드(jsonb)이고
어떤 것이 카운트(bigint)인지 알아야 비교를 컴파일할 수 있게 만든다.

#### 4f. `OPTIONAL MATCH` — 술어가 `ON` 으로 간다

```
A LEFT JOIN B ON true WHERE b.x = a.y
```
는 LEFT JOIN이 지키려던 행을 정확히 버린다. 그래서 OPTIONAL 절의 술어는
`OptionalScope` 에 모아 두었다가, **그 술어가 언급하는 마지막 별칭을 도입한 조인의 `ON`** 에 붙인다
([`compile.rs:130-142, 248-275`](../../engine/src/cypher/compile.rs)).

조인은 의존 순서로 방출되므로, 그 조인이 술어를 평가할 수 있는 가장 이른 지점이고
optional 쪽을 거르면서 행을 떨어뜨리지 않는 유일한 지점이다.

### [5]~[6] 실행

```rust
Spi::connect(|client| {
    let table = client.select(sql, None, &[JsonB(params.clone()).into()]);
    table.filter_map(|r| r.get::<JsonB>(1).ok().flatten().map(|j| j.0)).collect()
})
```
— [`cypher/mod.rs:145-152`](../../engine/src/cypher/mod.rs)

**모든 행이 `Vec<Value>` 로 메모리에 모인다.** `SetOfIterator` 는 그 벡터를 순회할 뿐이다
([`cypher/mod.rs:108`](../../engine/src/cypher/mod.rs)).
→ 스트리밍이 아니다. [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-11**

### [8] 감사

모든 호출이 `og_data.og_audit` 에 INSERT한다 — 읽기 질의도 포함
([`cypher/mod.rs:122-135`](../../engine/src/cypher/mod.rs)).
`.ok()` 로 오류를 무시하지만, 읽기 전용 트랜잭션에서 INSERT가 어떻게 처리되는지는
**미확인**이다 (**ARCH-03**).

---

## 3. 쓰기 질의 파이프라인

```
og_cypher('kb', $$ MATCH (a:Person) WHERE a.name = $n
                   CREATE (a)-[:KNOWS]->(b:Person {name: 'x'}) RETURN count(*) $$)
  │
  ├─[1] Ddl 절이 첫 번째면 → compat::ddl::run() 후 종료
  ├─[2] 읽기 부분(MATCH/UNWIND)만 골라 컴파일 → 바인딩 행들(envs)
  │       Compiler + build_select_pub + exec_json
  ├─[3] 바인딩 행마다 쓰기 절을 순서대로 적용
  │       Create  → create_pattern → storage::create_node_inner / create_edge_inner
  │       Merge   → 먼저 찾고, 없으면 만든다 (기존 바인딩은 고정점으로 핀 고정)
  │       Set/Remove → apply_set → storage::set_*_props_inner
  │       Delete  → storage::delete_node_inner / delete_edge_inner
  │       Return  → 행별로 평가해 out에 push
  └─[4] RETURN에 집계가 있으면 fold_aggregates(out) 로 접기
```

근거: [`engine/src/cypher/mod.rs:158-334`](../../engine/src/cypher/mod.rs).

### 쓰기 경로의 특별한 규칙 (Facts)

**라벨은 만들어진다, 거부되지 않는다.**
쓰기 시 존재하지 않는 라벨은 타입으로 **생성된다** — Neo4j가 하는 일이고,
Neo4j 자리를 대신하는 것이 목적이기 때문이다.
라벨 리스트는 왼쪽→오른쪽으로 넓은→좁은으로 읽는다:
`(:_Entity:Doc)` 에서 `Doc` 이 새 이름이면 `_Entity` 의 서브타입으로 선언된다
([`catalog/types.rs:202-231`](../../engine/src/catalog/types.rs)).

**노드는 라벨을 나중에 얻을 수 없다.**
타입이 식별자의 일부이므로, 타입을 바꾸면 정체성이 바뀌고 모든 인접 항목이 무효가 된다.
`SET n:Label` 은 이미 가진 라벨(자기 타입이나 그 위)이면 no-op, 아니면 명시적 오류다
([`cypher/mod.rs:604-634`](../../engine/src/cypher/mod.rs)).

**`REMOVE n:Old SET n:New` 는 클래스 이름 변경으로 인식된다.**
좁은 형태로만 적용된다: 제거 라벨이 실제 타입이고 추가 라벨이 비어 있을 때만.
타입 이름을 바꾸고, 별칭 뷰를 옮기고, `compat_index` 의 `type_name` 을 갱신하고,
스키마 버전을 올린다 ([`cypher/mod.rs:506-574`](../../engine/src/cypher/mod.rs)).

**집계는 나중에 접힌다.**
쓰기 경로는 바인딩을 한 행씩 걸으므로 행 간을 볼 수 없다.
`… RETURN count(n)` 의 인자를 행마다 보관했다가, 모든 행이 나온 뒤 `fold_aggregates` 가 접는다
([`cypher/mod.rs:322-384`](../../engine/src/cypher/mod.rs)).
지원 집계는 `count`, `collect`, `sum`, `avg`, `min`, `max` 뿐이다 —
그 외는 `Value::Null` 이 된다 ([`cypher/mod.rs:376`](../../engine/src/cypher/mod.rs)).

**쓰기 질의는 컴파일 캐시에 들어가지 않는다.**
`compile_cached` 가 쓰기 질의를 만나면 `Err("write queries are not compiled to a
single statement")` 를 낸다 ([`cypher/mod.rs:52-55`](../../engine/src/cypher/mod.rs)).

---

## 4. TypeQL 파이프라인

```
og_typeql('bookstore', $$ match $b isa book, has genre "science fiction";
                          fetch { "title": $b.title }; $$)
  │
  ├─ parser::parse_script(query)          → Vec<Query { stages }>
  ├─ is_write(q)?  Insert|Put|Delete|Update|Define|Undefine
  ├─ 읽기: Compiler::new(gid) → 스테이지를 감싸며 SQL 생성 → SPI
  └─ 쓰기: write::* 가 바인딩 행마다 절차 실행
```

근거: [`engine/src/typeql/mod.rs:1-65`](../../engine/src/typeql/mod.rs).

### Cypher와 공유하는 것 / 공유하지 않는 것 (Facts)

| | 공유? | 근거 |
|---|---|---|
| 저장 테이블 (`og_data.*`) | ✅ | `typeql/compile.rs:1-13` |
| 타입 카탈로그 (`og_catalog.*`) | ✅ | `use crate::catalog::{labeling, types}` |
| 구간 라벨 서브타입 판정 | ✅ | 같은 `labeling::og_subtypes` |
| 식별자 인코딩 | ✅ | `use crate::id` |
| 트랜잭션 | ✅ | 둘 다 호출자 트랜잭션 안 |
| **중간 표현(IR)** | ❌ | `crate::cypher` 참조 0건 |
| **`Compiler` / `Bind` 타입** | ❌ | 각 모듈에 독립 정의 |
| **`og_reach` 도달성 재작성** | ❌ | Cypher 컴파일러에만 존재 |
| **컴파일 캐시** | ❌ | `PLAN_CACHE` 는 `cypher/mod.rs` 전용 |
| **`ensure_view` 유니온 뷰** | ⚠️ 부분 | TypeQL은 `og_node`/`og_role_player` 를 직접 조인 |

`grep -rn "crate::cypher" engine/src/typeql/` → 0건.
`grep -rn "crate::typeql" engine/src/cypher/` → 0건.

→ [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-05**

### TypeQL의 세 가지 조인 (Facts)

```
isa   → og_data.og_node 을 구간 라벨이 돌려준 서브타입 집합으로 필터
has   → $has 인접 세그먼트 + 속성 타입 필터
        ★ 이 필터는 이웃 id의 시프트+마스크다 — 타입이 식별자 안에 있으므로 조인이 0회
role  → og_data.og_role_player, 역할 특수화까지 확장
```
— [`typeql/compile.rs:4-12`](../../engine/src/typeql/compile.rs)

### 파이프라인 스테이지는 **감싸기**로 구현된다

각 shaping 스테이지가 이전 것을 감싸는 서브쿼리가 된다.
그래서 `sort` 후 `limit` 과 `limit` 후 `sort` 가 하나로 정규화되지 않고
**진짜로 다른 SQL** 을 만든다 (spec 010 FR-036,
[`typeql/mod.rs:7-10`](../../engine/src/typeql/mod.rs)).

---

## 5. 캐시되는 것과 안 되는 것

| 항목 | 캐시? | 키 | 무효화 | 위험 |
|---|---|---|---|---|
| 파싱 + 컴파일 결과 | ✅ `PLAN_CACHE` | `(graph, query)` | **512개 초과 시 전량 폐기만** | 스키마 변경에 무효화되지 않음 → **ARCH-02** |
| 실행 계획 | ✅ PostgreSQL | 준비 문장 | PostgreSQL이 관리 | — |
| 타입 유니온 뷰 | ✅ `pg_class` 존재 여부 | 뷰 이름 | `drop_all_views()` | 캐시된 SQL이 폐기된 뷰를 참조할 수 있음 |
| 백엔드 로컬 CSR | ✅ `thread_local` | 없음 (하나만) | 수동 `og_csr_drop()` | 스냅샷 동결 |
| 쓰기 질의 컴파일 | ❌ | — | — | 매번 재컴파일 |
| TypeQL 컴파일 | ❌ | — | — | 매번 재컴파일 |
| `view_properties(tid)` | ❌ | — | — | 컴파일마다 SPI 조회 |
| `og_subtypes(tid)` | ❌ | — | — | 컴파일마다 SPI 조회 |

---

## Decisions

1. **컴파일 타깃은 SQL이지 함수 파이프라인이 아니다.** 조인 순서 규칙을 따로 만들지 않는다 —
   PostgreSQL이 우리보다 잘한다 (spec 003 plan.md 설계 결정 1).
2. **라벨은 컴파일 타임에 사라진다.** 실행 시점 계층 판정 비용은 0이다 (설계 결정 2).
3. **파라미터는 바인딩된다.** 비교 대상 컬럼의 선언 타입으로 캐스팅되어 인덱스가 살아 있고
   주입이 불가능하다 (설계 결정 4).
4. **도달성 재작성은 보수적이다.** 여기서 틀리면 타이밍이 아니라 **답이 바뀐다**
   ([`compile.rs:326-328`](../../engine/src/cypher/compile.rs) 주석).

## Facts

- 컴파일된 SQL은 `og_cypher_sql(graph, query)` 로 언제든 확인할 수 있다.
  `EXPLAIN` 에 붙여넣거나 자신의 SQL에 넣어도 동작한다 — 그냥 SQL이기 때문이다.
- `og_cypher_explain(graph, query, analyze)` 이 컬럼/SQL/플랜을 JSON으로 낸다
  ([`cypher/mod.rs:675-696`](../../engine/src/cypher/mod.rs)).
- `og_cypher_columns(query)` 는 **파싱만** 하고 DB에 접근하지 않는다
  (`immutable, parallel_safe`). `RETURN *` 이면 빈 리스트를 낸다 — 틀린 순서보다 낫다.

---

## Forbidden / Required

**Forbidden**
- 컴파일러가 사용자 값을 SQL 텍스트에 넣는 것. 반드시 `$1` jsonb를 거칠 것.
- `reachability_only` 판정을 넓히는 것. 넓히면 답이 바뀐다 — 성능이 아니라 정확성 문제다.
- 쓰기 절을 단일 SQL 문장으로 재작성하는 것 (트리거·CTE 부작용 의존이 된다).
- `PLAN_CACHE` 에 쓰기 질의를 넣는 것.

**Required**
- 컴파일 타임 결정이 카탈로그를 읽으면 **스키마 버전을 캐시 키에 포함**할 것.
- 새 Cypher 함수를 추가하면 `func()` 의 미지 함수 오류 메시지 목록도 갱신할 것
  ([`compile.rs:1558-1566`](../../engine/src/cypher/compile.rs)).
- 새 절/스테이지를 추가하면 `is_write()` 판정과 Bolt의 읽기/쓰기 분류를 함께 확인할 것.

<!-- affects: architecture, backend, api, llm -->
<!-- requires-update: 01_architecture/04_storage_architecture.md, 01_architecture/05_type_system_architecture.md, 02_api/, 03_backend/ -->
