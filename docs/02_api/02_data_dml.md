# 데이터 DML — 노드 · 엣지 생성 / 수정 / 삭제

> **이 문서가 답하는 질문**
> - 노드/엣지를 만들면 어떤 테이블이 몇 개 갱신되는가?
> - 선언하지 않은 프로퍼티를 쓰면 어떻게 되는가? (자동 승격 / text 확장)
> - `og_delete_node`가 반환하는 숫자는 무엇인가?
> - 64비트 식별자에 무엇이 들어 있는가?
> - 이 함수들이 실패하는 정확한 조건은?

---

## 1. 결정(Decision) — 쓰기 경로가 Rust에 있는 이유

읽기 경로는 **일부러** Rust에 없다. Cypher 컴파일러가 `og_data.og_adj`를 직접
건드리는 SQL을 방출해서 PostgreSQL 플래너가 순회 전체를 보도록 한다
([engine/src/storage/mod.rs:1](../../engine/src/storage/mod.rs#L1) 모듈 주석).

쓰기 경로는 반대다. **세 구조를 하나의 트랜잭션에서 보조를 맞춰 갱신**해야 하므로
Rust에 있다(spec 001 FR-012).

| 구조 | 테이블 |
|---|---|
| 레지스트리 | `og_data.og_node` / `og_data.og_edge` |
| 타입별 프로퍼티 테이블 | `og_data.n_<tid>` / `og_data.e_<tid>` |
| 양방향 인접 세그먼트 | `og_data.og_adj` (`dir = 'o'` 와 `'i'` 각각) |

---

## 2. 사실 — 프로퍼티 페이로드가 처리되는 방식

모든 값은 **바인딩된 jsonb 파라미터 하나**에서 추출된다. 사용자 값이 SQL 텍스트로
보간되는 경로는 없다(spec 003 FR-026,
[engine/src/storage/mod.rs:160](../../engine/src/storage/mod.rs#L160) `plan_props`).

선언된 프로퍼티는 실컬럼으로, 나머지는 `__ext` jsonb로 간다:

```text
declared "name"  →  p_name = ($2->>'name')::text
undeclared "tmp" →  __ext  = NULLIF($2 - ARRAY['name']::text[], '{}'::jsonb)
```

배열 타입은 `jsonb_array_elements_text` 경유로 캐스팅된다
([engine/src/storage/mod.rs:212](../../engine/src/storage/mod.rs#L212)).

### 2.1 쓰기 시점 프로퍼티 승격 (`declare_new_props`)

정의: [engine/src/storage/mod.rs:87](../../engine/src/storage/mod.rs#L87)

Cypher 애플리케이션은 아무것도 선언하지 않는다 — Neo4j에는 선언할 스키마가 없기
때문이다. 그래서 **쓰기 시점에** 새 프로퍼티를 실컬럼으로 승격한다.

| JSON 값 | 추론되는 컬럼 타입 |
|---|---|
| `true` / `false` | `bool` |
| 정수 | `int8` |
| 실수 | `float8` |
| 문자열 | `text` |
| 배열 · 객체 | **승격하지 않음** — `__ext`에 남음 ([storage/mod.rs:52](../../engine/src/storage/mod.rs#L52)) |

배열/객체를 승격하지 않는 이유: 여기서 중요한 배열 프로퍼티는 `embedding` 하나이고,
그건 `og_add_embedding`이 `vector(N)`으로 선언한다. 먼저 jsonb로 선언해 버리면 그
길이 막힌다([storage/mod.rs:47](../../engine/src/storage/mod.rs#L47) 주석).

**타입 충돌 시 동작**: 나중 값이 컬럼 타입과 맞지 않으면 거부하지 않고 **`text`로
단방향 확장(widening)** 한다. Neo4j는 노드마다 프로퍼티 타입이 다를 수 있고,
`text`는 그걸 모두 표현할 수 있는 유일한 컬럼 타입이며, 단방향이라 진동하지 않는다
([storage/mod.rs:78](../../engine/src/storage/mod.rs#L78) 주석).

확장 가능한 컬럼 타입은 추론으로 만들어진 것뿐이다:
`WIDENABLE = ["bool", "int8", "float8"]` ([storage/mod.rs:64](../../engine/src/storage/mod.rs#L64)).
숫자 간 확장(`float8 ← int8`, `numeric ← int8/float8`)은 확장 없이 그대로 수용된다
([storage/mod.rs:67](../../engine/src/storage/mod.rs#L67) `type_accepts`).

> **금지**: 명시적으로 `og_add_property(... 'int')`로 선언한 컬럼에
> 문자열을 쓰면 `text`로 확장되지 **않는다** — `WIDENABLE`에 `int4`가 없다.
> 캐스트 실패가 `node insert failed: …`로 표면화된다.

---

## 3. 노드

### `og_create_node(graph text, type_name text, props jsonb DEFAULT '{}') RETURNS int8`

정의: [engine/src/storage/mod.rs:246](../../engine/src/storage/mod.rs#L246) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값(`PARALLEL UNSAFE`)

**무엇을 하는가**: 노드 하나를 만들고 그 64비트 식별자를 반환한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 엔티티 타입 이름 |
| `props` | `jsonb` | 선택 | `'{}'` | 프로퍼티 객체. 선언된 것은 컬럼, 나머지는 `__ext` |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int8` | 아니오 | 새 노드 id (`og_make_id(0, type_id, local)`) |

**부수 효과 (순서대로)**
1. `og_data.og_id_alloc`에서 local id 할당 ([storage/mod.rs:23](../../engine/src/storage/mod.rs#L23)).
2. 미선언 프로퍼티 승격 (§2.1).
3. `INSERT INTO og_data.og_node (id, type_id)`.
4. `INSERT INTO og_data.n_<tid> (...)`.
5. `og_cypher_stats()`용 카운터: `nodes-created` +1, `labels-added` +1, `properties-set` += 키 개수.

**예제**

```sql
SELECT og_create_node('default', 'Person',
                      '{"name": "Aria", "born": 1990}'::jsonb);
-- 412316860417
```

**실패 조건**

| 입력 | 오류 | 위치 |
|---|---|---|
| 그래프 없음 | `graph '<g>' does not exist` | [types.rs:118](../../engine/src/catalog/types.rs#L118) |
| 타입 없음 | `type '<t>' does not exist. did you mean: …` | [types.rs:135](../../engine/src/catalog/types.rs#L135) |
| 관계/속성 타입 지정 | `'<type_name>' is not an entity type` | [storage/mod.rs:255](../../engine/src/storage/mod.rs#L255) |
| 추상 타입 | `'<type_name>' is abstract and cannot be instantiated` | [storage/mod.rs:258](../../engine/src/storage/mod.rs#L258) |
| 캐스트 불가한 값 / `NOT NULL` 위반 | `node insert failed: <postgres error>` | [storage/mod.rs:286](../../engine/src/storage/mod.rs#L286) |
| 타입당 local id 36비트 소진 | `local id <n> exhausted the 36-bit space for this type` | [id.rs:42](../../engine/src/id.rs#L42) |

---

### `og_set_node_props(id int8, props jsonb) RETURNS void`

정의: [engine/src/storage/mod.rs:293](../../engine/src/storage/mod.rs#L293) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 프로퍼티를 **병합**한다(전체 교체가 아니다). 선언 컬럼은 대입, 나머지는 `__ext ||` 로 합친다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `id` | `int8` | 필수 | — | 노드 id. 타입은 id에서 shift-and-mask로 얻는다 |
| `props` | `jsonb` | 필수 | — | 병합할 프로퍼티 |

**반환**: 없음.

**예제**

```sql
SELECT og_set_node_props(412316860417, '{"born": 1991, "city": "Seoul"}'::jsonb);
```

생성되는 SQL의 모양 ([storage/mod.rs:320](../../engine/src/storage/mod.rs#L320)):

```sql
UPDATE og_data.n_12
   SET p_born = ($2->>'born')::int4,
       __ext = COALESCE(__ext,'{}'::jsonb) || COALESCE(NULLIF($2 - ARRAY['born']::text[], '{}'::jsonb),'{}'::jsonb)
 WHERE id = $1
```

**실패 조건**
- `id`의 타입 비트가 저장 테이블 없는 타입을 가리킴 → `unknown type for id <id>`
  ([storage/mod.rs:300](../../engine/src/storage/mod.rs#L300))
- 캐스트 실패 / 제약 위반 → `update failed` 패닉
- **존재하지 않는 `id`는 오류가 아니다** — `UPDATE ... WHERE id = $1`이 0행을
  갱신하고 조용히 성공한다 → [12_improvements_api.md](12_improvements_api.md) API-03.

**주의**: 엣지 프로퍼티를 SQL에서 직접 고치는 공개 함수는 **없다**.
`set_edge_props_inner`는 Rust 내부용(`pub fn`, `#[pg_extern]` 아님,
[storage/mod.rs:328](../../engine/src/storage/mod.rs#L328))이고, Cypher `SET r.prop = …`
경로에서만 호출된다 → [12_improvements_api.md](12_improvements_api.md) API-03.

---

### `og_delete_node(id int8) RETURNS int8`

정의: [engine/src/storage/mod.rs:350](../../engine/src/storage/mod.rs#L350) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 노드와 **연결된 모든 엣지**를 지운다 (Cypher `DETACH DELETE` 의미론).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `id` | `int8` | 필수 | — | 노드 id |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int8` | 아니오 | 지운 엣지 수 + 1(노드 자신). 노드가 없어도 최소 `1`을 반환한다 |

**부수 효과** ([storage/mod.rs:355](../../engine/src/storage/mod.rs#L355)):
`og_data.og_adj`에서 인접 엣지 id를 모아 각각 `delete_edge_inner`,
그 다음 타입 테이블 행 · `og_data.og_node` 행 · `og_data.og_adj`의 `src = id` 행 삭제.

**예제**

```sql
SELECT og_delete_node(412316860417);   -- 4  (3 edges + the node)
```

**실패 조건**: 조회/삭제 SPI 실패 시 패닉(`node delete failed`, `registry delete failed`).
존재하지 않는 id는 오류가 아니며 `1`을 반환한다 —
`og_delete_edge`가 `0`을 반환하는 것과 비대칭 → [12_improvements_api.md](12_improvements_api.md) API-03.

---

## 4. 엣지

### `og_create_edge(graph text, rel_type text, src int8, dst int8, props jsonb DEFAULT '{}') RETURNS int8`

정의: [engine/src/storage/mod.rs:389](../../engine/src/storage/mod.rs#L389) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 엣지 하나를 만들고, 선언된 역할 제약을 검증하고, **인접 세그먼트 양방향**을 같은 트랜잭션에서 갱신한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `rel_type` | `text` | 필수 | — | 관계 타입 이름 |
| `src` | `int8` | 필수 | — | 출발 노드 id (`ordinal = 0` 역할의 플레이어) |
| `dst` | `int8` | 필수 | — | 도착 노드 id (`ordinal = 1` 역할의 플레이어) |
| `props` | `jsonb` | 선택 | `'{}'` | 엣지 프로퍼티 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int8` | 아니오 | 새 엣지 id |

**역할 검증** ([storage/mod.rs:455](../../engine/src/storage/mod.rs#L455)):
`og_supertypes(tid)` 전체에서 `player_type_id IS NOT NULL AND ordinal IN (0,1)`인
역할을 모아, `ordinal = 0`은 `src`의 타입을, `ordinal = 1`은 `dst`의 타입을
`og_is_subtype`으로 검사한다.

**부수 효과**
1. `INSERT INTO og_data.og_edge (id, type_id, src, dst)`
2. `INSERT INTO og_data.e_<tid> (...)`
3. `adjacency::append(src, tid, 'o', dst, eid)` — 정방향 세그먼트
4. `adjacency::append(dst, tid, 'i', src, eid)` — 역방향 세그먼트
5. 카운터: `relationships-created` +1, `properties-set` += 키 개수

인접 세그먼트는 힙 튜플 1개에 이웃 **최대 256개**를 담는다
(`CHUNK = 256`, [engine/src/storage/adjacency.rs:15](../../engine/src/storage/adjacency.rs#L15)).

**예제**

```sql
SELECT og_create_edge('default', 'ACTED_IN',
                      412316860417,      -- a Person
                      481036337153,      -- a Film
                      '{"role": "Neo"}'::jsonb);
```

**실패 조건**

| 입력 | 오류 | 위치 |
|---|---|---|
| 그래프/타입 없음 | `graph '…' does not exist` / `type '…' does not exist. did you mean: …` | — |
| 엔티티 타입 지정 | `'<rel_type>' is not a relation type` | [storage/mod.rs:411](../../engine/src/storage/mod.rs#L411) |
| 추상 관계 타입 | `'<rel_type>' is abstract and cannot be instantiated` | [storage/mod.rs:414](../../engine/src/storage/mod.rs#L414) |
| 역할 타입 위반 | `role '<name>' of relation '<rel>' requires a '<expected>', got '<got>'` | [storage/mod.rs:480](../../engine/src/storage/mod.rs#L480) |
| 프로퍼티 캐스트 실패 | `edge insert failed: <postgres error>` | [storage/mod.rs:442](../../engine/src/storage/mod.rs#L442) |

> ⚠️ **존재하지 않는 `src`/`dst`는 검증되지 않는다.** `id_type(src)`가 실재하지
> 않는 타입 id를 내면 `type_name_of`가 `type#<n>`을 돌려주고, 역할이 선언되지
> 않은 관계라면 아예 검사가 없다. 유령 엣지가 만들어질 수 있고,
> `og_check_integrity()`가 나중에 `orphan_node`로 잡는다
> → [12_improvements_api.md](12_improvements_api.md) API-07.

---

### `og_delete_edge(id int8) RETURNS int8`

정의: [engine/src/storage/mod.rs:496](../../engine/src/storage/mod.rs#L496) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 엣지 하나와 양방향 인접 항목, 역할 플레이어 행을 지운다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `id` | `int8` | 필수 | — | 엣지 id |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int8` | 아니오 | 지운 엣지 수. **존재하지 않으면 `0`** ([storage/mod.rs:514](../../engine/src/storage/mod.rs#L514)) |

**예제**

```sql
SELECT og_delete_edge(549755813889);   -- 1
SELECT og_delete_edge(1);              -- 0
```

**실패 조건**: SPI 실패 시 `edge lookup failed` / `edge delete failed` /
`registry delete failed` 패닉. 없는 id는 오류가 아니다.

---

### `og_add_role_player(graph text, rel_type text, edge_id int8, role text, player int8) RETURNS void`

정의: [engine/src/storage/mod.rs:531](../../engine/src/storage/mod.rs#L531) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: n-ary 관계에 세 번째 이상의 참여자를 붙인다(spec 002 FR-005). `og_data.og_role_player`에 행을 추가한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `rel_type` | `text` | 필수 | — | 관계 타입 이름 |
| `edge_id` | `int8` | 필수 | — | 대상 엣지(TypeQL에서는 reify된 관계 노드) id |
| `role` | `text` | 필수 | — | `og_add_role`로 선언된 역할 이름 |
| `player` | `int8` | 필수 | — | 참여자 노드 id |

**반환**: 없음. `ON CONFLICT DO NOTHING`이므로 멱등.

**예제**

```sql
SELECT og_add_role('default', 'PUBLISHING', 'publisher', 'Company', 2);
SELECT og_add_role_player('default', 'PUBLISHING', 549755813889,
                          'publisher', 618475290625);
```

**실패 조건**
- 역할 미선언 → `relation '<rel_type>' has no role named '<role>'` ([storage/mod.rs:542](../../engine/src/storage/mod.rs#L542))
- 플레이어 타입 위반 → `role '<role>' requires a '<expected>', got '<got>'` ([storage/mod.rs:547](../../engine/src/storage/mod.rs#L547))

읽기는 `og_typeql_role` 뷰로 한다 ([engine/sql/access.sql:324](../../engine/sql/access.sql#L324)).

---

## 5. 식별자 인코딩

```text
 bit 63        54           36                              0
 +---+----------+------------+-------------------------------+
 | 0 | shard: 9 | type_id:18 |          local_id: 36         |
 +---+----------+------------+-------------------------------+
```

[engine/src/id.rs:3](../../engine/src/id.rs#L3). 순회가 만지는 모든 것이 고정 폭
정수다 — 노드의 타입은 카탈로그 조인이 아니라 shift-and-mask다. shard 비트는
spec 007이 식별자를 다시 쓰지 않고 분산할 수 있도록 미리 예약해 둔 것이다.

| 상수 | 값 |
|---|---|
| `LOCAL_BITS` | 36 → 타입당 최대 `68,719,476,735` 인스턴스 |
| `TYPE_BITS` | 18 → 최대 `262,143` 타입 |
| `SHARD_BITS` | 9 → 최대 `511` 샤드 |

### `og_make_id(shard int4, type_id int4, local int8) RETURNS int8`

정의: [engine/src/id.rs:88](../../engine/src/id.rs#L88) · 휘발성: `IMMUTABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

**무엇을 하는가**: 세 조각을 하나의 식별자로 합친다. 범위를 벗어나면 조용히 잘리지 않고 오류를 낸다.

**실패 조건**

| 조건 | 오류 |
|---|---|
| `shard ∉ 0..511` | `shard id <n> out of range (0..511)` |
| `type_id ∉ 0..262143` | `type id <n> out of range (0..262143)` |
| `local ∉ 0..68719476735` | `local id <n> exhausted the 36-bit space for this type` |

### `og_id_type(id int8) RETURNS int4` / `og_id_shard(id int8) RETURNS int4` / `og_id_local(id int8) RETURNS int8`

정의: [engine/src/id.rs:73](../../engine/src/id.rs#L73), [:78](../../engine/src/id.rs#L78), [:83](../../engine/src/id.rs#L83) ·
휘발성: `IMMUTABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

**무엇을 하는가**: 식별자를 분해한다. 데이터베이스를 조회하지 않는다 — 순수 비트 연산.

**예제**

```sql
SELECT og_make_id(0, 12, 1) AS id,
       og_id_shard(og_make_id(0, 12, 1)) AS shard,
       og_id_type (og_make_id(0, 12, 1)) AS type_id,
       og_id_local(og_make_id(0, 12, 1)) AS local;
--        id | shard | type_id | local
-- 824633720833 |     0 |      12 |     1
```

**실패 조건**: 없음. `STRICT`이므로 `NULL` 입력은 `NULL` 출력.

---

## 6. 무결성 확인

```sql
SELECT * FROM og_check_integrity();
-- 빈 결과가 통과 조건
```

검사 항목은 [05_traversal_and_stats.md §5](05_traversal_and_stats.md) 참조.

---

## 7. 금지 / 필수

- **필수**: 프로퍼티 값은 반드시 `props jsonb` 인자로 넘길 것.
  질의 텍스트에 값을 연결하지 말 것.
- **필수**: 엣지를 만들기 전에 `src`/`dst` 노드가 실제로 존재하는지 호출측에서
  확인할 것 — 이 함수는 확인하지 않는다.
- **금지**: `og_data.og_adj`에 직접 `INSERT`/`DELETE` 하지 말 것.
  세그먼트 채움(`n`)과 배열 길이가 어긋나면 `og_check_integrity()`가
  `segment_length_mismatch`로 잡는다.
- **금지**: 반환값 `1`을 "실제로 지웠다"로 해석하지 말 것 —
  `og_delete_node`는 없는 id에도 `1`을 반환한다.
- **금지**: 대소문자만 다른 프로퍼티 이름을 한 타입에 섞어 쓰지 말 것(§2, `column_name` 규칙).

---

## 8. 관련 문서

- 타입·프로퍼티 선언 → [01_graph_ddl.md](01_graph_ddl.md)
- Cypher `CREATE` / `MERGE` / `SET` / `DELETE` → [03_cypher.md](03_cypher.md)
- 인접 세그먼트와 순회 → [05_traversal_and_stats.md](05_traversal_and_stats.md)
- 오류 체계 → [11_errors.md](11_errors.md)

<!-- affects: api, backend, data -->
<!-- requires-update: 02_api/03_cypher.md, 02_api/05_traversal_and_stats.md -->
