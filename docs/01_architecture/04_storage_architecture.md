# 저장 아키텍처 — CSR 세그먼트, 타입 테이블, 뷰

> **이 문서가 답하는 질문**
> - 인접 정보는 물리적으로 어떻게 배치되고, 왜 그 배치인가?
> - 프로퍼티는 어디에 있는가? 실컬럼과 `__ext` 의 경계는 무엇으로 결정되는가?
> - 생성되는 뷰가 두 종류인데, 각각 무엇이고 언제 만들어지고 언제 사라지는가?
> - 식별자 안에 왜 shard 비트가 있는가?

---

## 1. 스키마 두 개

```sql
CREATE SCHEMA og_catalog;   -- 타입 시스템 (spec 002)
CREATE SCHEMA og_data;      -- 그래프 저장 (spec 001)
```
— [`engine/sql/bootstrap.sql:13-17`](../../engine/sql/bootstrap.sql)

**모든 구조가 평범한 힙 릴레이션이다.**
그래서 MVCC / WAL / vacuum / `pg_dump` 를 공짜로 상속한다 (헌법 원칙 IX,
[`bootstrap.sql:8-10`](../../engine/sql/bootstrap.sql)).

### `og_catalog` — 카탈로그 테이블

| 테이블 | 무엇 |
|---|---|
| `graph` | 그래프 목록. 한 DB에 여러 독립 그래프 |
| `type` | 타입. `kind ∈ {'e','r','a'}`, `storage_table`, `iri` |
| `type_parent` | 상속 DAG. 다중 상속이면 타입당 여러 행 |
| `type_label` | **구간 라벨** `(lft, rgt, depth)`, 경로마다 `path_id` |
| `property` | 선언된 프로퍼티 → 물리 컬럼 매핑 |
| `role` | 관계 타입의 이름 있는 참여자 슬롯 (+ `parent_role_id` 특수화) |
| `og_constraint` | PostgreSQL 네이티브로 표현 불가한 제약 |
| `rule` | 관계 특성 (transitive/symmetric/reflexive/inverse) |
| `typeql_function` | TypeQL `fun` 원문 (평가되지 않음) |
| `schema_version` | 에이전트 스키마 캐시의 무효화 키 |
| `setting` | 런타임 설정 (chunk_size, genai.* 등) |
| `embedding` | 임베딩 슬롯 메타데이터 |
| `compat_index` | Neo4j 인덱스 이름 → (타입, 프로퍼티) 디렉터리 |
| `prefix` | RDF 네임스페이스 |
| `mapping` | 관계형 테이블 → 그래프 타입 매핑 |
| `agent_role` | 에이전트 역할과 한도 |

### `og_data` — 데이터 테이블

| 테이블 | 무엇 |
|---|---|
| **`og_adj`** | **CSR형 인접 세그먼트** — 이 프로젝트가 AGE가 아닌 이유 |
| `og_node` | 노드 레지스트리 `(id, type_id)` — 의도적으로 앙상함 |
| `og_edge` | 엣지 레지스트리 `(id, type_id, src, dst)` |
| `og_id_alloc` | 타입별 로컬 id 워터마크 |
| `og_role_player` | n-ary 롤 플레이어 |
| `og_history` | 시간 축 (valid_from/valid_to/recorded_at/txid/payload) |
| `og_source` | 출처 (source/ingested_at/confidence/author) |
| `og_audit` | 감사 로그 |
| `og_embedding_state` | 임베딩 원본 해시 (스테일 판정) |
| `og_iri`, `og_triple_overflow` | RDF 어댑터 지원 |
| `n_<tid>` / `e_<tid>` / `a_<tid>` | **타입별 스토리지 테이블** (런타임 생성) |
| `v_<tid>` / `ve_<tid>` | **타입 유니온 뷰** (컴파일 시 생성) |
| `"TypeName"` | **별칭 뷰** (타입 생성 시) |

---

## 2. CSR 인접 세그먼트 — `og_data.og_adj`

```sql
CREATE TABLE og_data.og_adj (
    src   int8   NOT NULL,
    etype int4   NOT NULL,
    dir   "char" NOT NULL,   -- 'o' outgoing | 'i' incoming
    seq   int4   NOT NULL,
    n     int4   NOT NULL,   -- live element count
    nbr   int8[] NOT NULL,
    eid   int8[] NOT NULL,
    PRIMARY KEY (src, etype, dir, seq)
) WITH (fillfactor = 80);

ALTER TABLE og_data.og_adj ALTER COLUMN nbr SET STORAGE MAIN;
ALTER TABLE og_data.og_adj ALTER COLUMN eid SET STORAGE MAIN;
```
— [`engine/sql/bootstrap.sql:197-211`](../../engine/sql/bootstrap.sql)

### 왜 이 모양인가

| 설계 요소 | 이유 | 근거 |
|---|---|---|
| 이웃 ≤ 256개/행 | 256 × 8B × 2배열 = 4KB → 8KB 힙 페이지 하나 안 | [`adjacency.rs:13-15`](../../engine/src/storage/adjacency.rs) |
| `nbr` + `eid` 정렬 배열 | `unnest(nbr, eid)` 로 한 번에 풀림 | [`access.sql:17-21`](../../engine/sql/access.sql) |
| `STORAGE MAIN` | TOAST되면 방금 산 지역성을 잃는다 | [`bootstrap.sql:208-209`](../../engine/sql/bootstrap.sql) |
| `(etype, dir)` 키 분할 | 타입/방향 가지치기가 공짜 (FR-003) | [`bootstrap.sql:193-194`](../../engine/sql/bootstrap.sql) |
| `seq` 분할 | 이웃 천만 개인 슈퍼노드를 256행씩 스트리밍 (FR-014, FR-020) | [`bootstrap.sql:194-195`](../../engine/sql/bootstrap.sql) |
| `fillfactor = 80` | append/remove가 UPDATE이므로 HOT 갱신 여유 | [`bootstrap.sql:206`](../../engine/sql/bootstrap.sql) |

### 쓰기 연산 (Facts)

**append** — 꼬리 세그먼트에 이어 붙이고, 꽉 차면 새 청크를 만든다:

```sql
UPDATE og_data.og_adj a
   SET nbr = a.nbr || $4::int8, eid = a.eid || $5::int8, n = a.n + 1
 WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::"char"
   AND a.seq = (SELECT max(seq) FROM og_data.og_adj WHERE …)
   AND a.n < $6
 RETURNING a.seq
```
— [`adjacency.rs:19-44`](../../engine/src/storage/adjacency.rs)

`RETURNING` 이 `None` 이면 `INSERT` 로 새 청크를 만든다.
**즉 append 한 번이 최대 SQL 2문장이다.**

**remove** — 같은 인덱스에서 두 배열을 잘라 붙여 정렬을 유지하고, 빈 세그먼트는 회수한다:

```sql
SET nbr = a.nbr[1 : i.idx - 1] || a.nbr[i.idx + 1 : array_length(a.nbr, 1)],
    eid = a.eid[1 : i.idx - 1] || a.eid[i.idx + 1 : array_length(a.eid, 1)],
    n   = a.n - 1
```
— [`adjacency.rs:48-72`](../../engine/src/storage/adjacency.rs)

`array_position(eid, $4)` 로 위치를 찾으므로 **세그먼트 내 선형 탐색**이다 (최대 256).

### 읽기 경로

읽기는 Rust를 거치지 않는다. 컴파일된 SQL이 직접 읽는다:

```sql
-- engine/sql/access.sql:14-22 — og_expand
SELECT u.nbr, u.eid
  FROM og_data.og_adj a, LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
 WHERE a.src = $1 AND a.dir = $3 AND ($2 IS NULL OR a.etype = ANY($2))
```

`LANGUAGE sql STABLE PARALLEL SAFE ROWS 50` — 인라인되어 플래너가 인접 스캔 자체를 본다.

> **`n` 과 배열 길이의 정합성**: `og_expand` 는 `unnest` 를 쓰므로 `n` 을 보지 않고,
> `og_degree` 는 `sum(n)` 을 본다. 두 경로가 어긋나는 것은 실제 위험이며,
> `og_check_integrity()` 의 3번 검사가 정확히 그것을 잡는다:
> `WHERE n <> COALESCE(array_length(nbr,1),0) OR array_length(nbr,1) <> array_length(eid,1)`
> ([`storage/stats.rs:224-241`](../../engine/src/storage/stats.rs)).
> 다만 이 검사는 **사후 진단**이지 강제가 아니다 — `CHECK` 제약이 아니다.

---

## 3. 앙상한 레지스트리

```sql
CREATE TABLE og_data.og_node (id int8 PRIMARY KEY, type_id int4 NOT NULL);
CREATE TABLE og_data.og_edge (id int8 PRIMARY KEY, type_id int4 NOT NULL,
                              src int8 NOT NULL, dst int8 NOT NULL);
```
— [`bootstrap.sql:227-241`](../../engine/sql/bootstrap.sql)

**의도적으로 앙상하다(deliberately SKINNY).** 프로퍼티 페이로드가 여기 없다.

- 존재 이유 1: 계층의 서브타입이 몇 개든 타입 스캔의 **앵커 릴레이션이 하나**다.
- 존재 이유 2: role/참조 검증이 인덱스 프로브 **한 번**이다.
- **순회는 이 테이블을 읽지 않는다.** `og_adj` 가 이웃 id와 엣지 id를 직접 나른다.

이것이 AGE의 `_ag_label_vertex` 와 다른 점이다
([`bootstrap.sql:216-225`](../../engine/sql/bootstrap.sql) 주석).

---

## 4. 타입 스토리지 테이블

### 생성

```sql
-- 엔티티
CREATE TABLE og_data.n_4 (id int8 PRIMARY KEY, __ext jsonb)
-- 관계
CREATE TABLE og_data.e_7 (id int8 PRIMARY KEY, src int8 NOT NULL, dst int8 NOT NULL, __ext jsonb)
```
— [`catalog/types.rs:414-417`](../../engine/src/catalog/types.rs)

추상 타입은 스토리지 테이블이 없다 (`storage_table IS NULL`),
그래서 인스턴스화하려 하면 `"'{name}' is abstract and cannot be instantiated"` 오류가 난다
([`storage/mod.rs:257-258, 413-414`](../../engine/src/storage/mod.rs)).

### 프로퍼티 → 컬럼

```
프로퍼티 "range_km"  →  column_name()  →  "p_range_km"
                        ALTER TABLE og_data.n_4 ADD COLUMN IF NOT EXISTS p_range_km int4
```
— [`catalog/types.rs:53-66, 550-553`](../../engine/src/catalog/types.rs)

`column_name()` 은 **결정적이고 주입 안전**하다:
`p_` 접두사 + ASCII 영숫자/`_` 는 소문자로, 그 외 유니코드 문자는 유지, 나머지는 `_`.

> 한글 프로퍼티 이름이 `_` 로 접히지 않는 이유가 여기 있다:
> `이름` 과 `용량` 이 같은 컬럼으로 접혀 두 프로퍼티가 조용히 합쳐지는 것을 막기 위해서다
> ([`catalog/types.rs:46-52`](../../engine/src/catalog/types.rs) 주석).

### `__ext` — 미선언 프로퍼티의 행선지

```rust
fn ext_expr(plan: &PropPlan, param: &str) -> String {
    // 선언된 이름들을 뺀 나머지 jsonb, 비면 NULL
    format!("NULLIF({param} - ARRAY[{list}]::text[], '{{}}'::jsonb)")
}
```
— [`storage/mod.rs:228-240`](../../engine/src/storage/mod.rs)

`__ext` 는 인덱스도 통계도 없다. **의도된 인센티브**다 — 선언하면 빠른 경로를 얻는다.

### 쓰기 시 프로퍼티 승격 (Decisions)

Cypher 앱은 아무것도 선언하지 않으므로, 이대로 두면 모든 프로퍼티가 `__ext` 에 간다.
그래서 쓰기 시점에 승격시킨다:

```rust
fn infer_column_type(v: &Value) -> Option<&'static str> {
    match v {
        Value::Bool(_)   => Some("bool"),
        Value::Number(n) => Some(if n.is_i64() || n.is_u64() { "int8" } else { "float8" }),
        Value::String(_) => Some("text"),
        _ => None,   // 배열/객체는 __ext 에 남는다
    }
}
```
— [`storage/mod.rs:53-60`](../../engine/src/storage/mod.rs)

**배열/객체가 승격되지 않는 이유가 중요하다**: 여기서 중요한 배열 프로퍼티는 `embedding` 이고,
그건 `og_add_embedding` 이 벡터 인덱스를 만들 때 `vector(N)` 으로 선언한다.
먼저 jsonb로 선언해 버리면 그 길이 막힌다 ([`storage/mod.rs:47-52`](../../engine/src/storage/mod.rs) 주석).

### 확장(widening) 규칙

값의 타입이 컬럼과 다르면 **`text` 로 단방향 확장**한다.
Neo4j는 프로퍼티가 노드마다 다른 타입을 가질 수 있고, `text` 는 그걸 모두 표현하는
유일한 컬럼 타입이며, 단방향이므로 진동하지 않는다.

**확장 대상은 우리가 추론으로 만들 수 있었던 타입뿐이다:**

```rust
const WIDENABLE: &[&str] = &["bool", "int8", "float8"];
```
— [`storage/mod.rs:62-64`](../../engine/src/storage/mod.rs)

`vector(1536)` 이나 `timestamptz` 는 **의도적으로** 선언된 것이므로 확장하지 않는다.
(2026-08-16에 이 가드 없이 벡터 스위트가 깨진 이력이 주석에 남아 있다,
[`storage/mod.rs:121-126`](../../engine/src/storage/mod.rs).)

확장은 **DDL이다**: 별칭 뷰를 떨어뜨리고 → `ALTER TABLE … ALTER COLUMN … TYPE text` →
별칭 뷰를 다시 만든다. **모든 서브타입 테이블에 대해** 반복한다
([`storage/mod.rs:127-153`](../../engine/src/storage/mod.rs)).
→ 쓰기 경로가 DDL을 실행한다는 뜻이다. [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-03**

---

## 5. 두 종류의 뷰 (혼동 주의)

### (a) 별칭 뷰 — `og_data."TypeName"`

```rust
CREATE VIEW og_data."MeetingRoom" AS SELECT * FROM og_data.n_12
```
— [`catalog/types.rs:89-98`](../../engine/src/catalog/types.rs)

- **언제**: 타입 생성 시, 이름 변경 시, 확장(widening) 후 복구 시.
- **왜**: psql이나 BI 도구가 `og_data.n_12` 대신 사람이 읽을 이름을 보게 하려고.
- **절대 치명적이지 않다**: 이름이 충돌하면 로그만 남기고 타입 생성은 계속된다.
- `drop_all_views()` 는 이것을 건드리지 않는다 (`v_%` / `ve_%` 만 매칭).

### (b) 타입 유니온 뷰 — `og_data.v_<tid>` / `og_data.ve_<tid>`

```sql
CREATE VIEW og_data.v_1 AS
  SELECT id, 1::int4 AS type_id, p_model, p_year, NULL::int4 AS p_range_km, __ext FROM og_data.n_1
  UNION ALL
  SELECT id, 4::int4, p_model, p_year, p_range_km, __ext FROM og_data.n_4
```
— [`cypher/views.rs:7-12, 102-138`](../../engine/src/cypher/views.rs)

- **언제**: **컴파일 시**, 해당 뷰가 없으면 생성 (`ensure_view`).
- **왜**: `MATCH (v:Vehicle)` 이 Vehicle·Car·EV를 **프로퍼티 컬럼과 함께** 한 릴레이션으로 봐야 한다.
- **후손 집합**은 구간 인덱스 범위 스캔 한 번으로 얻는다 (spec 002 FR-009).
- **컬럼 정렬**: 어떤 서브타입에 없는 프로퍼티는 `NULL::<type> AS <col>` 로 채운다.
- **구체 후손이 없는 추상 타입**: `SELECT NULL::… WHERE false` 로 모양만 맞춘 빈 릴레이션.
- **무효화**: `drop_all_views()` 가 `og_data` 의 `v\_%` / `ve\_%` 를 전부
  `DROP VIEW IF EXISTS … CASCADE` 한다. `bump_schema_version()` 이 이걸 부른다
  ([`labeling.rs:172-182`](../../engine/src/catalog/labeling.rs)).

**노드/엣지 판정이 개념적 `kind` 가 아니라 스토리지 테이블 이름으로 결정된다:**

```sql
AND ((NOT $2 AND (storage_table LIKE 'og_data.n\_%' OR storage_table LIKE 'og_data.a\_%'))
  OR ($2 AND storage_table LIKE 'og_data.e\_%'))
```
— [`cypher/views.rs:65-69`](../../engine/src/cypher/views.rs)

TypeQL이 관계를 reify하고 속성 인스턴스를 노드 테이블(`a_<tid>`)로 만들기 때문이다.
그것들은 Cypher에게 노드이고, 여기서 그렇게 말하는 것이 spec 010 FR-040을 참으로 만든다.

### 뷰 요약

| | 별칭 뷰 | 타입 유니온 뷰 |
|---|---|---|
| 이름 | `og_data."TypeName"` | `og_data.v_<tid>`, `og_data.ve_<tid>` |
| 생성 시점 | 타입 생성/개명/확장 시 | **Cypher 컴파일 시** |
| 만드는 코드 | `catalog/types.rs:89` | `cypher/views.rs:93` |
| 목적 | 사람/BI 도구 편의 | 라벨 → 구체 테이블 해소 |
| 실패 시 | 로그만, 계속 진행 | `error!` 로 중단 |
| 폐기 | 개명/확장 시 개별 | `drop_all_views()` 전량 |

---

## 6. 식별자

```text
 bit 63       54            36                               0
 +---+---------+-------------+--------------------------------+
 | 0 | shard:9 | type_id:18  |          local_id:36           |
 +---+---------+-------------+--------------------------------+
```
— [`engine/src/id.rs:1-29`](../../engine/src/id.rs)

| 필드 | 비트 | 최대 | 결과 |
|---|---:|---|---|
| shard | 9 | 511 | spec 007용 예약. **현재 항상 0** |
| type_id | 18 | 262,143 | `og_catalog.type_id_seq MAXVALUE 262143` 로 강제 |
| local_id | 36 | 68,719,476,735 | 타입당 인스턴스 상한 |

**얻는 것**: 노드의 타입이 시프트+마스크다. 카탈로그 조인이 아니다.
TypeQL의 `has` 조인에서 속성 타입 필터가 **조인 0회**가 되는 이유가 이것이다
([`typeql/compile.rs:8-10`](../../engine/src/typeql/compile.rs)).

**할당**: `og_data.og_id_alloc` 의 `INSERT … ON CONFLICT DO UPDATE SET next_id = next_id + 1
RETURNING next_id - 1` — 타입별 워터마크
([`storage/mod.rs:24-34`](../../engine/src/storage/mod.rs)).

**오버플로**: `make_id()` 가 범위를 벗어나면 `error!` 로 즉시 실패한다.
조용히 잘린 id가 저장소에 도달할 수 없다 ([`id.rs:31-45`](../../engine/src/id.rs)).

**shard 비트의 현재 상태 (Facts)**
- `alloc_id()` 는 언제나 `id::make_id(0, type_id, local)` 을 부른다.
- `with_shard()` 헬퍼는 존재하나 `engine/src/` 안에 호출자가 없다.
- `og_id_shard(id)` SQL 함수는 노출되어 있으나 항상 0을 낸다.
- `og_catalog.placement` 테이블은 부트스트랩 스키마에 **없다** (spec 007 plan.md가 설계로만 언급).
→ [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-13**

---

## 7. 백업 등록 — 조용한 함정을 막는 장치

`CREATE EXTENSION` 스크립트가 만든 테이블은 확장에 속하고,
`pg_dump` 는 그것들에 대해 `CREATE EXTENSION` 만 내보낸다 — **내용은 건너뛴다.**
그래서 사용자 데이터를 담는 모든 릴레이션을 configuration data로 등록해야 한다:

```sql
SELECT pg_catalog.pg_extension_config_dump('og_data.og_node', '');
SELECT pg_catalog.pg_extension_config_dump('og_data.og_adj', '');
… (총 26개 테이블 + 11개 시퀀스)
```
— [`bootstrap.sql:392-448`](../../engine/sql/bootstrap.sql)

**시퀀스도 등록된다** — 워터마크를 잃으면 이미 존재하는 식별자를 다시 발급하게 된다.

`og_catalog.setting` 은 부트스트랩이 4개 키를 심으므로 조건부 등록이다:

```sql
SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
    'WHERE key NOT IN (''chunk_size'', ''supernode_threshold'',
                       ''inference_max_depth'', ''schema_version'')');
```

**타입별 테이블(`n_*`, `e_*`, `a_*`)은 런타임 생성이라 평범한 사용자 테이블**이고
`pg_dump` 가 이미 커버한다.

> **주의**: 새 테이블을 `bootstrap.sql` 에 추가하고 등록을 빠뜨리면
> **덤프/복원이 조용히 빈 그래프를 복원한다.** 이 항목은 회귀 테스트가 필요하다.
> 현재 검증 여부는 미확인.

---

## 8. 통계와 정비

| 함수 | 무엇 | 위치 |
|---|---|---|
| `og_degree(src, etype, dir)` | `sum(n)` — 플래너가 확장 순서를 고를 때 쓰는 통계 | [`adjacency.rs:76-85`](../../engine/src/storage/adjacency.rs) |
| `og_degree_all(src, dir)` | 모든 타입 합계 | [`adjacency.rs:88-96`](../../engine/src/storage/adjacency.rs) |
| `og_graph_stats(graph)` | 그래프 통계 | [`storage/stats.rs:11`](../../engine/src/storage/stats.rs) |
| `og_degree_distribution(graph)` | 차수 분포 | [`storage/stats.rs:86`](../../engine/src/storage/stats.rs) |
| `og_reorganize(graph)` | 재조직 | [`storage/stats.rs:121`](../../engine/src/storage/stats.rs) |
| `og_check_integrity()` | 무결성 검사 — 복제 검증에도 쓰인다 | [`storage/stats.rs:172`](../../engine/src/storage/stats.rs) |

`og_check_integrity()` 는 spec 007 P0의 복제 검증 수단이기도 하다:
대기 서버에서 돌려 주 서버와 같은 결과가 나오는지 확인한다
([`specs/007-distributed-cluster/plan.md`](../../specs/007-distributed-cluster/plan.md)).

---

## Decisions

| # | 결정 | 대안 기각 이유 |
|---|---|---|
| D-1 | 인접을 CSR형 세그먼트로 저장 | 엣지당 1행 + B-tree는 홉마다 `degree`회 인덱스 조회를 만든다 (헌법 III) |
| D-2 | 배열을 `STORAGE MAIN` 으로 고정 | TOAST되면 지역성을 잃는다 |
| D-3 | 레지스트리를 앙상하게 유지 | 프로퍼티를 여기 두면 AGE의 `_ag_label_vertex` 가 된다 |
| D-4 | 선언 프로퍼티는 실컬럼 | JSONB는 컬럼 통계·인덱스·CHECK를 못 준다 (헌법 III 안티패턴) |
| D-5 | 쓰기 시 자동 승격 + text 단방향 확장 | Cypher 앱은 선언하지 않고, Neo4j는 타입 다형을 허용한다 |
| D-6 | 배열/객체는 승격하지 않는다 | `embedding` 이 `vector(N)` 으로 선언될 길을 막지 않기 위해 |
| D-7 | 의도적으로 선언된 타입은 확장 대상 제외 | 벡터/타임스탬프를 text로 바꾸면 의도를 파괴한다 |
| D-8 | shard 비트를 지금 예약 | 나중에 식별자를 다시 쓰지 않으려고 (spec 007 FR-008) |
| D-9 | 모든 사용자 데이터 릴레이션을 `pg_extension_config_dump` 등록 | 등록하지 않으면 덤프가 조용히 빈 그래프를 복원한다 |

## Facts

- `CHUNK = 256` 은 **Rust 상수가 유일한 진실**이다
  ([`adjacency.rs:15`](../../engine/src/storage/adjacency.rs)).
  `og_catalog.setting.chunk_size = '256'` 도 심겨 있지만
  ([`bootstrap.sql:256-260`](../../engine/sql/bootstrap.sql)),
  `engine/src/` 에서 이 키를 **읽는 코드가 없다** — `og_graph_stats` 조차
  Rust 상수를 보고한다 ([`storage/stats.rs:68, 77`](../../engine/src/storage/stats.rs)).
  즉 `chunk_size`, `supernode_threshold`, `inference_max_depth` 세 키는
  현재 **동작하지 않는 설정 손잡이**다. → **ARCH-14**
- `og_edge` 에 `src`/`dst` 인덱스가 있다 (`og_edge_src_idx`, `og_edge_dst_idx`).
  순회는 이것을 쓰지 않지만 role 검증과 정합성 검사가 쓴다.
- `og_role_player` 에는 `player_id` 인덱스만 있다 (기본키가 `(edge_id, role_id, player_id)`).

---

## Forbidden / Required

**Forbidden**
- ❌ 프로퍼티를 `og_node` / `og_edge` 레지스트리에 추가하는 것.
- ❌ `og_adj` 의 `nbr`/`eid` 를 `STORAGE EXTENDED`(기본값)로 되돌리는 것.
- ❌ 인접 갱신을 비동기로 만드는 것 (헌법 원칙 IX 안티패턴).
- ❌ `WIDENABLE` 밖의 타입을 자동 확장하는 것.
- ❌ 그래프 데이터를 위한 **별도 백업 경로**를 만드는 것 (헌법 원칙 I 안티패턴).

**Required**
- ✅ `bootstrap.sql` 에 사용자 데이터 테이블을 추가하면 **반드시**
  `pg_extension_config_dump` 로 등록할 것. 시퀀스도 마찬가지다.
- ✅ 새 타입 스토리지 테이블 접두사를 도입하면 `cypher/views.rs:65-69` 의
  노드/엣지 판정 패턴을 함께 갱신할 것.
- ✅ 쓰기 경로가 DDL을 실행하는 새 경로를 추가하지 말 것.
  이미 있는 것(승격/확장)도 락 영향을 문서화할 것.
- ✅ 인접 구조를 바꾸면 `og_check_integrity()` 의 검사 항목도 함께 갱신할 것.

<!-- affects: architecture, data, backend, operations -->
<!-- requires-update: 06_data/, 03_backend/, 01_architecture/05_type_system_architecture.md, 08_operations/ -->
