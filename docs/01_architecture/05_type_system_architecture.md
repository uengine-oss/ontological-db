# 타입 시스템 아키텍처 — 상속 DAG, 구간 라벨, 재라벨링 경계

> **이 문서가 답하는 질문**
> - `MATCH (v:Vehicle)` 이 왜 `Car`, `EV` 까지 찾으면서도 비용이 늘지 않는가?
> - 다중 상속에서도 왜 여전히 범위 비교 한 번인가?
> - 재라벨링(`og_relabel`)은 언제 일어나고, 무엇을 무너뜨리는가?
> - role 제약은 언제, 어디서 강제되는가?

---

## 1. 왜 타입 시스템이 여기 있는가

헌법 원칙 IV: **레이블은 문자열 태그가 아니라 타입 시스템의 시민이다.**

Neo4j의 멀티라벨은 `Car ISA Vehicle` 을 표현하지 못한다.
"Vehicle을 찾으면 Car도 나온다"를 하려면 애플리케이션이 두 번째 라벨을 손으로 유지해야 한다.
TypeDB는 그것을 표현하지만 PostgreSQL 밖에 있다.

이 프로젝트의 선택: **PostgreSQL 카탈로그 안에 상속 DAG를 두고,
서브타입 판정을 인덱스 범위 비교 한 번으로 만든다.**

그리고 헌법 원칙 IV는 대안을 **금지**한다:
> 상속 질의는 상수 시간이어야 한다. … 런타임 재귀 CTE로 계층을 펼치는 방식은 금지한다.

---

## 2. 카탈로그 구조

```
og_catalog.type          (type_id PK, graph_id, name, kind, is_abstract, storage_table, iri)
     │
     ├── og_catalog.type_parent  (type_id, parent_id)     ← 상속 DAG. 다중 상속이면 여러 행
     │
     ├── og_catalog.type_label   (type_id, path_id, graph_id, lft, rgt, depth)
     │        ★ 루트에서 오는 경로마다 한 행
     │
     ├── og_catalog.property     (prop_id, type_id, name, data_type, column_name, …)
     │
     ├── og_catalog.role         (role_id, rel_type_id, name, player_type_id,
     │                            ordinal, card_min, card_max, parent_role_id)
     │
     ├── og_catalog.og_constraint (con_id, type_id, kind, target, params)
     └── og_catalog.rule          (rule_id, rel_type_id, characteristic, target_type_id)
```
— [`engine/sql/bootstrap.sql:29-156`](../../engine/sql/bootstrap.sql)

### `kind` 세 값

| 값 | 의미 | 스토리지 접두사 |
|---|---|---|
| `'e'` | entity | `og_data.n_<tid>` |
| `'r'` | relation | `og_data.e_<tid>` |
| `'a'` | attribute (TypeQL) | `og_data.a_<tid>` |

`CHECK (kind IN ('e', 'r', 'a'))` 로 강제된다.

### `type_id` 는 재사용되지 않는다

`type_id` 가 모든 노드/엣지 식별자에 **박혀 있으므로**, 전역 유일해야 하고
절대 재사용되어서는 안 된다 ([`bootstrap.sql:31-33`](../../engine/sql/bootstrap.sql) 주석).
시퀀스가 `MAXVALUE 262143` (18비트)으로 제한된다.

---

## 3. 구간 라벨 — 이 시스템의 핵심 장치

### 개념

```
Vehicle  lft 1 · rgt 12
   ├── Car    lft 2 · rgt 7
   │     └── EV  lft 3 · rgt 6
   └── Truck  lft 8 · rgt 11
```

```text
X ⊑ Y  ⟺  Y.lft ≤ X.lft  AND  X.rgt ≤ Y.rgt
```

`EV ⊑ Vehicle?` → `1 ≤ 3 AND 6 ≤ 12` → 참. **인덱스 범위 비교 한 번.**
계층이 얼마나 깊든 넓든 상관없다.

지원 인덱스:
```sql
CREATE INDEX type_label_range_idx ON og_catalog.type_label (graph_id, lft, rgt);
CREATE INDEX type_label_lft_idx   ON og_catalog.type_label (graph_id, lft);
```
— [`bootstrap.sql:78-80`](../../engine/sql/bootstrap.sql)

### 라벨 배분 알고리즘

DFS로 `(lft, rgt)` 를 매기되 **`GAP = 1024` 간격**으로 벌린다:

```rust
let lft = *cursor;   *cursor += GAP;
for child in children { assign(…); }
let rgt = *cursor;   *cursor += GAP;
```
— [`catalog/labeling.rs:78-110`](../../engine/src/catalog/labeling.rs)

**간격의 목적**: 부모와 자식 사이에 타입을 끼워 넣을 때, 같은 자리에 1024번 삽입할 때까지는
빈 공간을 소비할 뿐 전체를 재번호하지 않는다 (spec 002 FR-013).

> `labeling.rs:112-116` 의 주석은 `insert_between` 이 흔한 경우를 처리한다고 말하지만,
> 현재 파일에 `insert_between` 함수는 존재하지 않는다. 실제로는
> `relabel_graph()` 전량 재계산이 유일한 경로다. → **ARCH-15**

### 다중 상속

**루트에서 오는 경로마다 라벨 행이 하나씩** 생긴다 (`path_id` 0..n):

```rust
let path_id = path_counter.entry(id).or_insert(0);
out.push((id, *path_id, lft, rgt, depth));
*path_id += 1;
```
— [`catalog/labeling.rs:105-108`](../../engine/src/catalog/labeling.rs)

`PRIMARY KEY (type_id, path_id)` 로 여러 행이 허용된다.
그래서 다중 상속에서도 판정이 **여전히 범위 비교**다 — 조건을 만족하는 라벨 행이
하나라도 있으면 참이다 (`EXISTS`).

### 사이클 검출

`relabel_graph()` 는 두 겹으로 사이클을 잡는다:

1. DFS 중 스택에 이미 있는 타입을 다시 만나면 `error!("inheritance cycle detected at type {id}")`
   ([`labeling.rs:87-89`](../../engine/src/catalog/labeling.rs))
2. 루트에서 도달 불가능한 노드가 있으면 사이클이다:
   `error!("inheritance cycle detected involving type(s) {orphans:?}")`
   ([`labeling.rs:127-144`](../../engine/src/catalog/labeling.rs))
3. BFS 워크에 `guard > dag.len() * 64` 안전장치

### SQL 표면

세 함수가 컴파일러와 access.sql이 쓰는 판정 API다:

| 함수 | 무엇 | 휘발성 |
|---|---|---|
| `og_subtypes(type_id)` | 자기 + 모든 후손 id | `stable, parallel_safe, strict` |
| `og_supertypes(type_id)` | 자기 + 모든 조상 id | `stable, parallel_safe, strict` |
| `og_is_subtype(sub, sup)` | 상수 시간 판정 | `stable, parallel_safe, strict` |

— [`catalog/labeling.rs:188-244`](../../engine/src/catalog/labeling.rs)

`access.sql` 의 `og_subtype_ids(root)` 도 같은 조인을 `LANGUAGE sql` 로 제공한다
([`access.sql:43-51`](../../engine/sql/access.sql)) —
"재귀가 완전히 없다는 점에 주목"하라고 주석이 명시한다.

---

## 4. 컴파일 타임 소멸

**라벨은 실행 시점에 존재하지 않는다.**

```
MATCH (v:Vehicle)
  │ [컴파일 타임]
  ├─ resolve_label_set → LabelMatch::Type(1)
  ├─ og_subtypes(1) → [1, 2, 3, 4]        ← 범위 스캔 1회
  ├─ ensure_view(1, false)
  │     CREATE VIEW og_data.v_1 AS
  │       SELECT … FROM og_data.n_1 UNION ALL
  │       SELECT … FROM og_data.n_2 UNION ALL …
  └─ FROM og_data.v_1 n1
  │ [런타임]
  └─ 플래너가 각 구체 테이블의 통계를 개별적으로 본다. 계층 비용 0.
```

관계 타입도 마찬가지로 배열 리터럴이 된다:

```rust
for t in &rel.types {
    if let Some(tid) = types::try_type_id(self.gid, t) {
        ids.extend(crate::catalog::labeling::og_subtypes(tid));
    }
}
// → "ARRAY[7,9,12]::int4[]"
```
— [`cypher/compile.rs:833-849`](../../engine/src/cypher/compile.rs)

**예외 — 런타임 판정이 남는 곳**: 변수가 jsonb로 도착했을 때
(프로시저 yield, `UNWIND` 통과 등)는 컴파일 타임에 타입을 모르므로
`og_is_subtype({alias}.type_id, {w})` 술어를 남긴다
([`cypher/compile.rs:743-747`](../../engine/src/cypher/compile.rs)).
여전히 인덱스 범위 비교 한 번이므로 원칙 IV 위반은 아니다.

---

## 5. 라벨 집합 해소 — Neo4j와 다른 지점

Neo4j에서 노드는 독립적인 라벨 여러 개를 가질 수 있다.
여기서는 **노드가 타입 하나를 갖고 그 위의 이름들을 상속한다.**

두 모델이 정확히 일치하는 경우는 **패턴의 라벨들이 하나의 사슬을 이룰 때**다:
`(:_Entity:Doc)` 은 `Doc` 타입인 노드다 — `Doc` 이 이미 `_Entity` 이기 때문이다.

```rust
// 가장 구체적인 라벨 = 나머지 전부의 서브타입인 것
let most_specific = ids.iter().copied()
    .find(|c| ids.iter().all(|o| c == o || labeling::og_is_subtype(*c, *o)));
```
— [`catalog/types.rs:178-188`](../../engine/src/catalog/types.rs)

| 결과 | 의미 | 컴파일러 처리 |
|---|---|---|
| `LabelMatch::Any` | 라벨 없음 | `og_data.og_node` 전체 |
| `LabelMatch::Type(t)` | 가장 구체적인 타입 | `og_data.v_<t>` |
| `LabelMatch::Nothing` | 만족 불가 (서로 무관한 라벨들) | `false` 술어 |

**존재하지 않는 라벨은 오류가 아니다.**
Cypher에서 그것은 단순히 아무것도 매칭하지 않으며,
라벨을 만들기 전에 존재를 확인하는 호출자가 그 동작에 의존한다.
철자 힌트는 **`NOTICE`** 로 나간다 — 합법적인 질의를 실패로 바꾸지 않으면서
오타를 찾을 수 있게 (spec 008 FR-008)
([`catalog/types.rs:160-175`](../../engine/src/catalog/types.rs)).

**`LabelMatch::Nothing` 의 정확한 처리가 중요하다**: OPTIONAL MATCH 아래에서는
질의 전체를 비우는 것이 아니라 **그 바인딩만** 비운다 → 조인에 NULL 행이 남는다
([`cypher/compile.rs:698-715, 777-792`](../../engine/src/cypher/compile.rs)).

### 쓰기 경로는 다르다

쓰기에서 모르는 라벨은 **거부되지 않고 생성된다** — Neo4j가 하는 일이고,
Neo4j 자리를 대신하는 것이 목적이기 때문이다.

라벨 리스트는 왼쪽→오른쪽으로 **넓은 → 좁은** 으로 읽는다:
`(:_Entity:Doc)` 에서 `Doc` 이 새 이름이면 `_Entity` 의 서브타입으로 선언된다.
"라벨 리스트가 계층을 나른다"고 읽을 수 있는 유일한 해석이고,
모든 `(:Super:Sub)` 작성자가 이미 쓰는 형태다
([`catalog/types.rs:202-231`](../../engine/src/catalog/types.rs)).

---

## 6. 노드는 타입을 바꿀 수 없다

**타입이 식별자의 일부이므로, 타입을 바꾸면 정체성이 바뀌고
그 노드를 가리키는 모든 인접 항목이 무효가 된다.**

```rust
error!(
    "cannot add label '{l}' to this node: a node's type is part of its \
     identifier here, so it cannot gain one after creation. To rename a \
     class, write `REMOVE n:Old SET n:New` — that is applied to the type. \
     To move a node between types, create it under the target type."
);
```
— [`cypher/mod.rs:627-632`](../../engine/src/cypher/mod.rs)

**예외 — no-op**: 이미 가진 라벨(자기 타입이거나 그 위)을 다시 붙이는 것은 오류가 아니다.
쓰기마다 재태깅하는 코드가 흔하기 때문 ([`cypher/mod.rs:604-626`](../../engine/src/cypher/mod.rs)).

**클래스 이름 변경**: `REMOVE n:Old SET n:New` 를 좁은 형태로만 인식해
**타입 자체의 이름을 바꾼다** — 모든 인스턴스에 대해, 상수 시간에, 식별자를 유지한 채.

적용 조건 (하나라도 어긋나면 거부):
- `REMOVE` 라벨이 정확히 1개, `SET` 라벨도 정확히 1개
- 두 변수 이름이 같음
- 제거 라벨이 **실제 존재하는 타입**
- 추가 라벨이 **아직 없는 이름** (둘 다 있으면 타입 간 이동이지 개명이 아니다)

부수 작업: 별칭 뷰 이동, `compat_index.type_name` 갱신, 스키마 버전 증가
([`cypher/mod.rs:506-574`](../../engine/src/cypher/mod.rs)).

---

## 7. 재라벨링 경계 — `og_relabel` / `bump_schema_version`

### 언제 라벨이 다시 계산되는가

`relabel_graph(graph_id)` 는 **그래프 전체**를 재계산한다:
`DELETE FROM og_catalog.type_label WHERE graph_id = $1` → DFS 재배분 → 행별 INSERT.

호출 지점: `og_relabel(graph_id)` 수동 호출, 그리고 타입 계층을 바꾸는 카탈로그 연산
(정확한 호출 지점 목록은 [`catalog/types.rs`](../../engine/src/catalog/types.rs) 참조).

**성능 특성 (Facts)**: 라벨 INSERT가 **타입 하나당 SQL 문장 하나**다
([`labeling.rs:160-167`](../../engine/src/catalog/labeling.rs)).
타입 1,000개 그래프의 재라벨링은 SQL 1,001문장이다. → **ARCH-15**

### `bump_schema_version` 이 무너뜨리는 것

```rust
pub fn bump_schema_version(graph_id: i32, description: &str) {
    // Generated per-type union views encode the descendant set, so any schema
    // change invalidates them (spec 003 / cypher::views).
    crate::cypher::views::drop_all_views();
    Spi::run_with_args("INSERT INTO og_catalog.schema_version …");
}
```
— [`catalog/labeling.rs:172-182`](../../engine/src/catalog/labeling.rs)

**타입 유니온 뷰가 후손 집합을 인코딩하고 있으므로, 어떤 스키마 변경이든 그것들을 무효화한다.**
`drop_all_views()` 는 `og_data` 의 `v\_%` / `ve\_%` 를 전부
`DROP VIEW IF EXISTS … CASCADE` 한다 ([`cypher/views.rs:158-177`](../../engine/src/cypher/views.rs)).

### 호출 지점 (Facts)

```
labeling.rs:169         relabel_graph 끝
catalog/types.rs:317    create graph
catalog/types.rs:599    add property
catalog/types.rs:653    add role
catalog/types.rs:681    add rule
cypher/mod.rs:572       rename Old -> New
typeql/schema.rs:126    typeql define
```

**주의**: `og_add_property` 가 여기 있다는 것은,
**Cypher 쓰기 하나가 새 프로퍼티를 승격시키면 그래프의 모든 유니온 뷰가 날아간다**는 뜻이다
([`storage/mod.rs:112-119`](../../engine/src/storage/mod.rs) → `og_add_property` →
[`catalog/types.rs:599`](../../engine/src/catalog/types.rs) → `bump_schema_version`).

그리고 `PLAN_CACHE` 는 이 신호를 받지 않는다. → **ARCH-02**

---

## 8. role 강제 — 언제, 어디서

### 스키마

```sql
CREATE TABLE og_catalog.role (
    role_id        int4 PRIMARY KEY,
    rel_type_id    int4 NOT NULL,
    name           text NOT NULL,
    player_type_id int4,          -- 이 슬롯에 들어갈 수 있는 타입
    ordinal        int4 NOT NULL, -- 0 = src, 1 = dst, 2.. = n-ary
    card_min, card_max,
    parent_role_id int4,          -- 역할 특수화 (spec 010 FR-009)
    UNIQUE (rel_type_id, name)
);
```
— [`bootstrap.sql:107-121`](../../engine/sql/bootstrap.sql)

### 강제 지점 (Facts)

**엣지 생성 시** — `validate_roles(tid, rel_type, src, dst)`:

```rust
"SELECT name, ordinal, player_type_id FROM og_catalog.role
  WHERE rel_type_id = ANY($1) AND player_type_id IS NOT NULL AND ordinal IN (0,1)"
// $1 = og_supertypes(tid)   ← 조상의 role 도 상속된다
```
그리고 `id::id_type(src)` / `id::id_type(dst)` 가 `player_type_id` 의 서브타입인지
`og_is_subtype` 로 검사한다. 위반 시:
```
role '{name}' of relation '{rel_type}' requires a '{expected}', got '{got}'
```
— [`storage/mod.rs:454-484`](../../engine/src/storage/mod.rs)

**n-ary 롤 플레이어 추가 시** — `og_add_role_player`:
같은 검사를 `ordinal` 제한 없이 수행하고, `og_data.og_role_player` 에
`ON CONFLICT DO NOTHING` 으로 삽입한다 ([`storage/mod.rs:530-559`](../../engine/src/storage/mod.rs)).

### 강제되지 않는 것 (Facts)

`grep` 으로 각 카탈로그 테이블의 **소비자**를 추적한 결과:

| 제약 | 저장됨 | 강제/적용됨 | 근거 |
|---|---|---|---|
| `role.player_type_id` (ordinal 0/1) | ✅ | ✅ 엣지 생성 시 | [`storage/mod.rs:474-483`](../../engine/src/storage/mod.rs) |
| `role.player_type_id` (ordinal 2+) | ✅ | ✅ `og_add_role_player` 시 | [`storage/mod.rs:544-552`](../../engine/src/storage/mod.rs) |
| `role.card_min` / `card_max` | ✅ | ❌ **강제되지 않음** — 읽는 곳은 `og_schema()` 표시용 하나뿐 | [`agent/mod.rs:158`](../../engine/src/agent/mod.rs) |
| `og_constraint` | ✅ | ⚠️ **TypeQL 경로에서만** 읽힌다 (`kind='value'`, `owns` 주석) | [`typeql/schema.rs:318, 562`](../../engine/src/typeql/schema.rs), [`typeql/write.rs:266`](../../engine/src/typeql/write.rs) |
| `og_catalog.rule` | ✅ | ❌ **어디서도 SELECT되지 않는다** | INSERT만: [`catalog/types.rs:675`](../../engine/src/catalog/types.rs), [`adapters/rdf.rs:434`](../../engine/src/adapters/rdf.rs) |

**두 개의 실질적 공백**

1. **`og_catalog.rule` 은 쓰기 전용이다.** `og_add_rule()` 과 RDF 적재가
   `transitive` / `symmetric` / `inverse` 특성을 기록하지만, `engine/src/` 어디에서도
   그 테이블을 `SELECT` 하지 않는다. 즉 **관계 특성 추론(spec 002 FR-027..FR-030)이
   질의에 반영되지 않는다.** `og_catalog.setting.inference_max_depth` 가 심겨 있는데도
   읽히지 않는 이유가 여기 있다. → **ARCH-16**
2. **role 카디널리티가 강제되지 않는다.** `card_min`/`card_max` 는 카탈로그에 저장되고
   `og_schema()` 가 에이전트에게 보고하지만, 엣지 생성/삭제 어디에서도 검사되지 않는다.
   에이전트는 "강제되는 제약"으로 읽을 것이다. → **ARCH-16**

`og_constraint` 는 Cypher 경로에서 읽히지 않는다 — TypeQL 스키마 왕복과
`owns` 주석 재현에만 쓰인다. Cypher로 쓴 데이터는 `required`/`key` 제약의
영향을 받지 않는다.

---

## 9. 스키마 버전 — 에이전트 캐시 무효화 키

```sql
CREATE TABLE og_catalog.schema_version (
    version int8 PRIMARY KEY, graph_id int4, changed_at timestamptz, description text);
```

에이전트가 스키마를 캐시하므로, 이것이 그들의 무효화 키다 (spec 002 FR-026, spec 008 FR-005).
`og_schema(graph, token_budget)` 이 현재 최대 버전을 함께 낸다
([`agent/mod.rs:24-30`](../../engine/src/agent/mod.rs)).

---

## Decisions

| # | 결정 | 대안 기각 이유 |
|---|---|---|
| D-1 | 상속을 구간(nested-set) 라벨로 인코딩 | 런타임 재귀 CTE는 헌법 원칙 IV가 금지 |
| D-2 | 다중 상속은 경로당 라벨 행 하나 | 하나의 라벨로는 DAG를 구간으로 표현할 수 없다 |
| D-3 | 라벨 간격 `GAP = 1024` | 중간 삽입마다 전체 재번호하면 대형 온톨로지가 무너진다 |
| D-4 | 라벨은 컴파일 타임에 소멸시킨다 | 런타임 판정을 남기면 플래너가 구체 테이블 통계를 못 본다 |
| D-5 | 노드는 타입을 바꿀 수 없다 | 타입이 식별자의 일부라 정체성이 바뀌고 인접이 전부 무효가 된다 |
| D-6 | `REMOVE n:Old SET n:New` 를 타입 개명으로 인식 | 그 질의의 의도를 상수 시간에, 식별자 유지한 채 달성하는 유일한 방법 |
| D-7 | 존재하지 않는 라벨은 `NOTICE` + 빈 결과 | Cypher 의미론. 합법 질의를 실패로 바꾸지 않는다 |
| D-8 | 스키마 변경 시 유니온 뷰 전량 폐기 | 뷰가 후손 집합을 인코딩하므로 부분 무효화가 더 위험하다 |

## Facts

- `type_id` 는 재사용되지 않으며 18비트(최대 262,143)로 제한된다.
- role은 조상 관계 타입에서 **상속된다** (`rel_type_id = ANY(og_supertypes(tid))`).
- 라벨 재계산은 **그래프 단위 전량 재계산**이며 부분 갱신 경로가 현재 없다.
- `insert_between` 은 주석에만 언급되고 코드에 존재하지 않는다.

---

## Forbidden / Required

**Forbidden**
- ❌ 상속 판정을 런타임 재귀 CTE로 하는 것 (헌법 원칙 IV 안티패턴, spec 002 SC-003이
  `EXPLAIN` 출력에 `Recursive` 노드가 없음을 검사한다).
- ❌ `type_id` 를 재사용하는 것. 식별자에 박혀 있다.
- ❌ 노드의 타입을 사후에 바꾸는 경로를 만드는 것.
- ❌ 존재하지 않는 라벨을 읽기 경로에서 오류로 만드는 것 (Cypher 의미론 위반).

**Required**
- ✅ 계층을 바꾸는 모든 연산은 `bump_schema_version()` 을 호출할 것 —
  그것이 유니온 뷰 무효화의 유일한 경로다.
- ✅ 새 제약 종류를 `og_constraint.kind` 에 추가하면 **강제 지점을 함께 구현**할 것.
  스키마에만 있고 강제되지 않는 제약은 거짓 문서를 만든다.
- ✅ 라벨 배분 알고리즘을 바꾸면 다중 상속(경로당 행)과 사이클 검출을 함께 검증할 것.

<!-- affects: architecture, data, backend, llm -->
<!-- requires-update: 06_data/, 01_architecture/03_query_pipeline.md, 01_architecture/08_improvements_architecture.md -->
