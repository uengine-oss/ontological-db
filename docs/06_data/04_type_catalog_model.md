# 04. 타입 카탈로그 모델 — DAG, 다중 상속, 구간 라벨

> **이 문서가 답하는 질문**
> - 타입 계층은 트리인가 DAG인가? 다중 상속은 어떻게 저장되는가?
> - 구간(nested-set) 라벨의 수학적 성질은 정확히 무엇인가?
> - 라벨은 언제, 얼마나 비싸게 다시 계산되는가?
> - 추상 타입의 `storage_table`이 NULL이라는 규칙이 무엇을 파생시키는가?

**정본**: [`engine/src/catalog/labeling.rs`](../../engine/src/catalog/labeling.rs) (250줄),
[`engine/src/catalog/types.rs`](../../engine/src/catalog/types.rs) (711줄),
[`engine/sql/bootstrap.sql:51-80`](../../engine/sql/bootstrap.sql).

---

## 사실 — 계층은 DAG다

`og_catalog.type_parent`는 `(type_id, parent_id)` 쌍의 집합이고, PK가 그 쌍이다
(`engine/sql/bootstrap.sql:52-56`). 한 타입이 여러 행을 가지면 다중 상속이다.
주석이 명시한다: "Multiple inheritance => several rows per type. (FR-003)"
(`engine/sql/bootstrap.sql:51`).

**사이클은 금지된다.** 검출은 라벨링 시점에 두 겹으로 이뤄진다:
1. DFS 중 스택 재방문 (`engine/src/catalog/labeling.rs:87-89`)
2. 루트에서 도달 불가능한 노드 탐지 —
   "그건 사이클일 때만 생긴다"(`engine/src/catalog/labeling.rs:127-144`)

두 번째 검사는 루트 없는 순환 컴포넌트를 잡는 안전망이다.
DFS만으로는 루트에서 시작하지 못하는 사이클을 못 본다.

**부모와 자식의 `kind`는 같아야 한다**:
```rust
if pk != k {
    error!("type '{name}' ({kind}) cannot inherit from '{p}' of kind '{pk}'");
}
```
(`engine/src/catalog/types.rs:386-388`)
엔티티는 엔티티만, 관계는 관계만, 속성은 속성만 상속한다.

---

## 사실 — 구간 라벨의 수학

정의:
```text
X ⊑ Y  ⟺  ∃ 라벨행 lx ∈ label(X), ly ∈ label(Y).
          ly.graph_id = lx.graph_id ∧ ly.lft ≤ lx.lft ∧ lx.rgt ≤ ly.rgt
```
(`engine/sql/bootstrap.sql:59-67`, `engine/src/catalog/labeling.rs:1-12`)

이것이 SQL로는:
```sql
SELECT EXISTS (
   SELECT 1 FROM og_catalog.type_label d, og_catalog.type_label a
    WHERE d.type_id = $1 AND a.type_id = $2
      AND a.graph_id = d.graph_id
      AND d.lft >= a.lft AND d.rgt <= a.rgt)
```
(`engine/src/catalog/labeling.rs:235-240`)

**재귀가 없다.** 이것이 헌법 원칙 IV(질의 시점 계층 재귀 금지)를 만족시키는 방식이다.

### 라벨 배정 알고리즘

깊이 우선 순회로 `lft`를 찍고 자식을 다 돈 뒤 `rgt`를 찍는다.
```rust
let lft = *cursor;  *cursor += GAP;
for child in children { assign(...); }
let rgt = *cursor;  *cursor += GAP;
```
(`engine/src/catalog/labeling.rs:92-103`)

- 커서는 1에서 시작한다(`engine/src/catalog/labeling.rs:146`).
- `GAP = 1024`(`engine/src/catalog/labeling.rs:23`). 인접한 두 라벨 사이에
  1,023개의 빈 값을 남긴다.
- 자식은 `type_id` 오름차순으로 순회한다(`engine/src/catalog/labeling.rs:96-97`) —
  결정적(deterministic) 라벨링을 위해서다.
- 루트는 부모가 없는 타입이고, `type_id` 오름차순으로 순회한다
  (`engine/src/catalog/labeling.rs:124-125, 150-152`).

### 성질 1 — 구간의 포함관계가 곧 계보다

`lft < rgt`가 CHECK 제약으로 강제된다(`engine/sql/bootstrap.sql:76`).
한 서브트리의 모든 후손 구간은 조상 구간에 완전히 포함된다.
`GAP` 덕분에 구간은 **접하지 않고 떨어져 있다** — 두 형제의 구간 사이에도 여유가 있다.

### 성질 2 — 다중 상속은 라벨 행이 여러 개다

```rust
let path_id = path_counter.entry(id).or_insert(0);
out.push((id, *path_id, lft, rgt, depth));
*path_id += 1;
```
(`engine/src/catalog/labeling.rs:105-107`)

**부모가 N개면 그 타입은 DFS에서 N번 방문되고, 라벨 행이 N개 생긴다.**
`path_id`는 0(주 경로)부터 세는 방문 순번이다(`engine/sql/bootstrap.sql:70`).

이 구조 덕분에 다중 상속이 **여전히 범위 비교 하나**로 풀린다 —
어느 한 경로에서라도 포함관계가 성립하면 서브타입이다.
`og_subtypes` / `og_supertypes`가 `DISTINCT`를 붙이는 이유가 이것이다
(`engine/src/catalog/labeling.rs:197, 217`):

```sql
SELECT DISTINCT d.type_id
  FROM og_catalog.type_label a
  JOIN og_catalog.type_label d
    ON d.graph_id = a.graph_id AND d.lft >= a.lft AND d.rgt <= a.rgt
 WHERE a.type_id = $1
```

**대가**: 다중 상속이 깊고 넓으면 라벨 행 수가 **경로 수만큼 지수적으로** 늘 수 있다.
DAG의 타입 수가 아니라 **루트→타입 경로 수**가 `type_label`의 행 수다.
`og_type_view`가 `path_id = 0` 행만 조인해 대표 깊이를 보여주는 이유다
(`engine/sql/access.sql:97`).

### 성질 3 — `graph_id` 비정규화

`type_label.graph_id`는 `type.graph_id`의 복사본이다(`engine/sql/bootstrap.sql:71`).
`type` 테이블과 조인하지 않고도 그래프 경계 안에서만 비교하기 위한 것이다.
`type_label_range_idx (graph_id, lft, rgt)`의 선두 컬럼이기도 하다.

**함의**: 그래프별 커서가 매번 1부터 시작하므로(`engine/src/catalog/labeling.rs:146`)
서로 다른 그래프의 `lft`/`rgt`는 **겹친다**. `graph_id` 조건을 빼먹은 질의는
다른 그래프의 타입을 서브타입으로 판정한다. 모든 라벨 질의가 `graph_id` 등가 조건을
포함해야 하는 이유다.

---

## 결정 — 라벨링은 항상 전체 재계산이다

```rust
pub fn relabel_graph(graph_id: i32) {
    let dag = load_dag(graph_id);            // 타입 전체 + 부모관계 전체를 메모리로
    ...
    Spi::run_with_args("DELETE FROM og_catalog.type_label WHERE graph_id = $1", ...);
    for (type_id, path_id, lft, rgt, depth) in out {
        Spi::run_with_args("INSERT INTO og_catalog.type_label (...) VALUES ($1,...,$6)", ...);
    }
    bump_schema_version(graph_id, "relabel");
}
```
(`engine/src/catalog/labeling.rs:117-170`)

**비용 구조**
| 단계 | 비용 |
|---|---|
| `load_dag` | `og_catalog.type` 전체 스캔 + `type_parent` 전체 스캔 (그래프 단위) |
| DFS | O(경로 수) — 노드 수가 아니라 경로 수 |
| `DELETE` | 그래프의 라벨 행 전부 |
| `INSERT` | **라벨 행 하나당 SPI 문장 하나** |
| `bump_schema_version` | 생성된 `v_*` / `ve_*` 뷰 **전부 드롭** (`engine/src/catalog/labeling.rs:175`) |

증분 경로는 **없다**. 소스 주석은 있다고 말한다:

> "For large ontologies this is the fallback path — [`insert_between`] handles the
> common case without touching existing labels."
> (`engine/src/catalog/labeling.rs:114-116`)

**`insert_between`은 저장소 어디에도 존재하지 않는다.**
(확인: `grep -rn "insert_between" engine/` → 위 주석 한 줄만 매치.)
`GAP = 1024`는 그 미구현 최적화를 위해 남겨둔 여유 공간이며, 현재는
소비되지 않는다 — 매번 커서가 1부터 다시 시작하기 때문이다. → `DATA-10`

### 언제 재라벨링이 일어나는가

| 호출부 | 근거 |
|---|---|
| `og_create_type` / `create_type_inner` 끝 | `engine/src/catalog/types.rs:451` |
| `og_drop_type` 끝 | `engine/src/catalog/types.rs:710` |
| TypeQL `$has` 내부 타입 최초 생성 | `engine/src/typeql/schema.rs:550` |
| `og_relabel(graph_id)` 수동 호출 | `engine/src/catalog/labeling.rs:247-249` |

**Cypher가 모르는 라벨을 쓸 때마다 타입이 생기고**
(`engine/src/catalog/types.rs:210-231`), 그때마다 전체 재라벨링 + 전체 뷰 드롭이 일어난다.
정착된 스키마에서는 문제가 아니지만, 스키마리스로 시작하는 워크로드에서는
초반 쓰기가 그래프 크기와 무관하게 카탈로그 비용을 반복 지불한다.

---

## 결정 — 상속은 "컬럼 복사"다

부모의 프로퍼티는 자식 테이블에 **실제 컬럼으로 복제된다.**

```rust
fn copy_inherited_properties(parent: i32, child: i32, child_table: &str) {
    // WITH RECURSIVE anc(type_id) AS (SELECT $1 UNION
    //   SELECT tp.parent_id FROM og_catalog.type_parent tp JOIN anc ON anc.type_id = tp.type_id)
    // SELECT p.name, p.data_type, p.column_name, p.required, p.is_key
    //   FROM og_catalog.property p JOIN anc ON anc.type_id = p.type_id
    for (name, dtype, col, required, is_key) in props {
        ALTER TABLE {child_table} ADD COLUMN IF NOT EXISTS {col} {dtype}
        ...
        INSERT INTO og_catalog.property (...) ON CONFLICT (type_id, name) DO NOTHING
    }
}
```
(`engine/src/catalog/types.rs:455-506`)

- 자식 테이블에 컬럼이 생기고,
- `og_catalog.property`에 **자식 소유의 행이 하나 더 생긴다.**

즉 `og_catalog.property`는 "선언"이 아니라 "유효한 프로퍼티의 전개(materialised)"다.
`plan_props`가 `WHERE type_id = $1` 하나로 그 타입의 모든 프로퍼티를 얻는 이유다
(`engine/src/storage/mod.rs:163-166`) — 조상을 거슬러 올라갈 필요가 없다.

**나중에 부모에 프로퍼티를 더하면?** `og_add_property`가 `og_subtypes(tid)`를 돌며
모든 후손 테이블에 컬럼을 추가하고 카탈로그 행을 넣는다
(`engine/src/catalog/types.rs:548-590`).

> **주의 — 원칙의 경계**: `copy_inherited_properties`는 `WITH RECURSIVE`로
> `type_parent`를 거슬러 올라간다(`engine/src/catalog/types.rs:459-463`).
> 헌법 원칙 IV가 금지하는 것은 **질의 시점**의 계층 재귀이고, 이것은 DDL 시점이다.
> 그래서 위반이 아니다. 다만 "이 저장소에 계층 재귀 SQL이 전혀 없다"는 진술은 거짓이다.

---

## 사실 — 추상 타입의 `storage_table`은 NULL이다

```rust
// Storage table. Abstract types get none — they are never instantiated.
if !is_abstract && k != 'a' { ... CREATE TABLE ... ; set storage_table ... }
```
(`engine/src/catalog/types.rs:410-442`)

두 종류가 `storage_table = NULL`이다:
1. `is_abstract = true`인 타입
2. **`kind = 'a'`(속성) 타입** — Cypher 경로로 만들어진 것

(2)는 미묘하다. TypeQL 경로로 만들어진 속성 타입은
`engine/src/typeql/schema.rs:270-278`에서 `og_data.a_<tid>` 테이블을 얻는다.
그러니 "속성 타입에는 테이블이 없다"는 참이 아니고, **"`og_create_type(..., 'attribute')`로
만든 속성 타입에는 없다"**가 참이다.

### NULL이 파생시키는 규칙

| 지점 | 동작 |
|---|---|
| 인스턴스 생성 | `error!("'{type_name}' is abstract and cannot be instantiated")` (`engine/src/storage/mod.rs:257-258`, `413-414`) |
| 프로퍼티 추가 | `if let Some(table) = storage_table(sub)` — 조용히 건너뛴다 (`engine/src/catalog/types.rs:549`) |
| 인덱스 생성 | 동일하게 건너뜀 (`engine/src/catalog/types.rs:609`) |
| RLS 활성화 | 동일 (`engine/src/interop/mod.rs:23`) |
| 이력 활성화 | 동일 (`engine/src/agent/mod.rs:453`) |
| 합집합 뷰 | `storage_table IS NOT NULL` 필터 (`engine/src/cypher/views.rs:65-70`) |
| 구체 후손이 없는 추상 타입의 뷰 | 형태만 맞는 **빈 관계** (`SELECT ... WHERE false`) (`engine/src/cypher/views.rs:121-133`) |

마지막 항목이 좋은 설계다. `MATCH (v:AbstractThing)`이 오류가 아니라 0행을 낸다.

### 뷰가 어떤 테이블을 고르는가는 이름 패턴이 정한다

```sql
AND ((NOT $2 AND (storage_table LIKE 'og_data.n\_%' OR storage_table LIKE 'og_data.a\_%'))
  OR ($2 AND storage_table LIKE 'og_data.e\_%'))
```
(`engine/src/cypher/views.rs:65-70`)

**개념적 `kind`가 아니라 저장 테이블 이름의 접두사가 노드/엣지 여부를 정한다.**
주석이 그 이유를 밝힌다 — TypeQL은 관계를 노드로 reify하고 속성을 노드 테이블에
materialise하며, 그것들은 Cypher에게 **노드**다(`engine/src/cypher/views.rs:60-64`).

**결과**: `og_data.n_*` / `e_*` / `a_*` 이름 규칙은 장식이 아니라 **의미론적 계약**이다.
저장 테이블 이름을 바꾸면 Cypher가 그 타입을 못 본다.

---

## 사실 — 카탈로그 변경이 무효화하는 것

```rust
pub fn bump_schema_version(graph_id: i32, description: &str) {
    crate::cypher::views::drop_all_views();
    INSERT INTO og_catalog.schema_version (version, graph_id, description) ...
}
```
(`engine/src/catalog/labeling.rs:172-182`)

`drop_all_views()`는 `og_data` 스키마의 `v\_%` / `ve\_%` 뷰를 **전부** `DROP ... CASCADE`한다
(`engine/src/cypher/views.rs:159-177`). 그래프 단위가 아니라 **DB 단위**다.
한 그래프에 타입을 하나 추가하면 다른 그래프의 생성 뷰도 같이 사라진다.

뷰는 다음 질의 때 `ensure_view()`가 다시 만든다(`engine/src/cypher/views.rs:93-97`).
"존재 여부"가 곧 "신선함"인 구조다 — 단순하고, 무효화 누락 버그가 원천 봉쇄된다.

호출자 목록:
| 호출부 | 근거 |
|---|---|
| `relabel_graph` | `engine/src/catalog/labeling.rs:169` |
| `og_create_graph` | `engine/src/catalog/types.rs:317` |
| `og_add_property` | `engine/src/catalog/types.rs:599` |
| `og_add_role` | `engine/src/catalog/types.rs:653` |
| `og_add_rule` | `engine/src/catalog/types.rs:681` |
| `og_map_table` | `engine/src/interop/mod.rs:114` (직접 `drop_all_views()`) |
| `og_materialize_mapping` | `engine/src/interop/mod.rs:166` (직접) |

---

## 금지 / 필수

**금지**
- `og_catalog.type_label`을 직접 수정하는 것. 유일한 정당 경로는 `og_relabel(graph_id)`.
- 라벨 질의에서 `graph_id` 등가 조건을 생략하는 것. 그래프 간 구간이 겹친다.
- 타입 계층에 사이클을 만드는 것. `relabel_graph`가 `error!`로 트랜잭션을 중단시킨다.
- `og_data`의 저장 테이블 이름을 `n_` / `e_` / `a_` 접두사에서 벗어나게 바꾸는 것.
- 부모와 다른 `kind`로 상속시키는 것 (엔진이 거부한다).

**필수**
- 스키마가 클수록(수천 타입) 타입 생성을 **배치로** 묶을 것.
  타입 하나마다 전체 재라벨링 + 전체 뷰 드롭이 돈다.
- 다중 상속을 도입할 때는 `type_label` 행 수를 확인할 것:
  ```sql
  SELECT graph_id, count(*) AS label_rows,
         count(DISTINCT type_id) AS types,
         round(count(*)::numeric / count(DISTINCT type_id), 2) AS paths_per_type
    FROM og_catalog.type_label GROUP BY graph_id;
  ```
  `paths_per_type`이 급격히 커지면 DAG가 다이아몬드로 폭발하고 있다는 신호다.
- 애플리케이션이 라벨을 동적으로 생성하지 않게 할 것 → [`02`](02_identifier_encoding.md).

---

<!-- affects: data, backend -->
<!-- requires-update: docs/06_data/05_property_model.md, docs/06_data/10_improvements_data.md -->
