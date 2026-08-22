# 그래프 · 타입 · 프로퍼티 · 역할 DDL

> **이 문서가 답하는 질문**
> - 그래프와 타입을 어떻게 만들고 지우는가?
> - 프로퍼티를 선언하면 물리적으로 무슨 일이 일어나는가?
> - `role`과 `rule`은 무엇을 강제하는가?
> - 서브타입 판정은 왜 재귀 없이 상수 시간인가?
> - DDL 함수가 실패하는 정확한 조건은?

---

## 1. 사실 — DDL이 만드는 물리 구조

타입 하나를 선언하면 다음이 **동시에** 생성된다
([engine/src/catalog/types.rs:410](../../engine/src/catalog/types.rs#L410)).

| 산출물 | 형태 | 조건 |
|---|---|---|
| 카탈로그 행 | `og_catalog.type` | 항상 |
| 물리 저장 테이블 | 엔티티 `og_data.n_<type_id>`, 관계 `og_data.e_<type_id>` | `is_abstract = false` 이고 `kind <> 'attribute'` |
| 별칭 뷰 | 타입 이름을 그대로 쓴 뷰 | 저장 테이블이 생길 때 |
| 인덱스 | 관계 테이블에 `(src)`, `(dst)` | `kind = 'relation'` |
| id 할당기 행 | `og_data.og_id_alloc` | 항상 |
| 구간 라벨 재계산 | `og_catalog.type_label` (`lft`/`rgt`) | 항상 (`relabel_graph`) |

엔티티 테이블의 기본 형태:

```sql
CREATE TABLE og_data.n_45 (id int8 PRIMARY KEY, __ext jsonb)
```

관계 테이블:

```sql
CREATE TABLE og_data.e_51 (id int8 PRIMARY KEY, src int8 NOT NULL,
                           dst int8 NOT NULL, __ext jsonb)
```

**결정(Decision)**: 물리 테이블 이름을 `n_<type_id>`로 두고 사람이 읽을 이름은
**뷰**로 제공한다. 타입 이름 변경이 상수 시간이 되고, 식별자가 이동하지 않는다.
비용은 `\dt`에 `n_45`가 보인다는 것 — 그래서 뷰를 만든다
([engine/src/catalog/types.rs:422](../../engine/src/catalog/types.rs#L422) 주석).

---

## 2. 그래프

### `og_create_graph(name text) RETURNS int4`

정의: [engine/src/catalog/types.rs:300](../../engine/src/catalog/types.rs#L300) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값(`PARALLEL UNSAFE`)

**무엇을 하는가**: 그래프 네임스페이스를 만들고 `graph_id`를 돌려준다. 이미 있으면 기존 id를 반환한다(멱등).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `name` | `text` | 필수 | — | 그래프 이름. 전역 유일 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int4` | 아니오 | `og_catalog.graph.graph_id` |

**예제**

```sql
SELECT og_create_graph('meeting');
```

**실패 조건**: 실질적으로 없음 — 존재하면 조회해서 반환한다
([engine/src/catalog/types.rs:302](../../engine/src/catalog/types.rs#L302)).
카탈로그 INSERT가 실패하면 `graph insert failed` 패닉이 `ERROR`로 올라온다.

---

### `og_drop_graph(name text) RETURNS void`

정의: [engine/src/catalog/types.rs:321](../../engine/src/catalog/types.rs#L321) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값

**무엇을 하는가**: 그래프에 속한 모든 타입의 저장 테이블을 `DROP TABLE ... CASCADE` 하고 `og_catalog.graph` 행을 지운다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `name` | `text` | 필수 | — | 그래프 이름 |

**반환**: 없음(`void`).

**예제**

```sql
SELECT og_drop_graph('meeting');
```

**실패 조건**
- 그래프가 없으면: `graph '<name>' does not exist`
  ([engine/src/catalog/types.rs:118](../../engine/src/catalog/types.rs#L118)).
- 저장 테이블 DROP 실패 시 `drop table failed` 패닉.

**주의(확인된 동작)**: `og_data.og_node` / `og_data.og_edge` / `og_data.og_adj`의
잔여 행은 **삭제하지 않는다** — `og_drop_type`은 지우지만
([engine/src/catalog/types.rs:704](../../engine/src/catalog/types.rs#L704)),
`og_drop_graph`는 저장 테이블과 `graph` 행만 지운다
([engine/src/catalog/types.rs:334](../../engine/src/catalog/types.rs#L334)).
`og_check_integrity()`의 `orphan_node`가 남을 수 있다.
→ [12_improvements_api.md](12_improvements_api.md) API-07.

---

## 3. 타입

### `og_create_type(graph text, name text, kind text, parents text[] DEFAULT '{}', is_abstract bool DEFAULT false) RETURNS int4`

정의: [engine/src/catalog/types.rs:348](../../engine/src/catalog/types.rs#L348) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 타입을 선언하고 저장 테이블·별칭 뷰·id 할당기를 만든 뒤 계층 라벨을 재계산한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `name` | `text` | 필수 | — | 타입 이름 (그래프 내 유일) |
| `kind` | `text` | 필수 | — | `entity`\|`e`\|`node` / `relation`\|`r`\|`rel`\|`edge` / `attribute`\|`a`\|`attr` ([engine/src/catalog/types.rs:374](../../engine/src/catalog/types.rs#L374)) |
| `parents` | `text[]` | 선택 | `'{}'` | 상위 타입 이름 목록. 두 개 이상이면 다중 상속 |
| `is_abstract` | `bool` | 선택 | `false` | `true`면 저장 테이블을 만들지 않고 인스턴스화 불가 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int4` | 아니오 | `type_id` |

**예제** (실제 [examples/demo.sql:19](../../examples/demo.sql#L19))

```sql
-- Abstract root: never instantiated, but queryable.
SELECT og_create_type('default', 'Work',   'entity', ARRAY[]::text[], true);
SELECT og_create_type('default', 'Film',   'entity', ARRAY['Work']);
SELECT og_create_type('default', 'AnimatedFilm', 'entity', ARRAY['Film']);
SELECT og_create_type('default', 'ACTED_IN', 'relation');
```

**실패 조건**

| 입력 | 오류 메시지 | 위치 |
|---|---|---|
| 그래프 없음 | `graph '<g>' does not exist` | [types.rs:118](../../engine/src/catalog/types.rs#L118) |
| 같은 이름 타입 존재 | `type '<name>' already exists in graph '<graph>'` | [types.rs:372](../../engine/src/catalog/types.rs#L372) |
| `kind`가 목록 밖 | `unknown type kind '<k>' (entity \| relation \| attribute)` | [types.rs:378](../../engine/src/catalog/types.rs#L378) |
| 부모 타입 없음 | `type '<p>' does not exist. did you mean: …` | [types.rs:135](../../engine/src/catalog/types.rs#L135) |
| 부모 종류 불일치 | `type '<name>' (<kind>) cannot inherit from '<p>' of kind '<pk>'` | [types.rs:387](../../engine/src/catalog/types.rs#L387) |
| `type_id`가 18비트 초과 | `type id <n> out of range (0..262143)` | [engine/src/id.rs:39](../../engine/src/id.rs#L39) |

**필수 규칙**: `parents`의 부모는 **같은 `kind`** 여야 한다. 엔티티가 관계를
상속할 수 없다.

---

### `og_drop_type(graph text, name text, cascade bool DEFAULT false) RETURNS void`

정의: [engine/src/catalog/types.rs:685](../../engine/src/catalog/types.rs#L685) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 타입과 **모든 서브타입**을 지운다. 인스턴스가 남아 있으면 `cascade => true` 없이는 거부한다(spec 002 FR-024).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `name` | `text` | 필수 | — | 타입 이름 |
| `cascade` | `bool` | 선택 | `false` | `true`면 인스턴스가 있어도 삭제 |

**반환**: 없음.

**삭제되는 것** ([engine/src/catalog/types.rs:700](../../engine/src/catalog/types.rs#L700)):
서브타입별 저장 테이블(`DROP TABLE ... CASCADE`), `og_data.og_node`,
`og_data.og_edge`, `og_data.og_adj`(`etype` 기준), `og_catalog.type` 행.
마지막에 `relabel_graph(gid)`로 구간 라벨을 재계산한다.

**예제**

```sql
SELECT og_drop_type('meeting', 'Reservation', cascade => true);
```

**실패 조건**
- 타입 없음 → `type '<name>' does not exist. did you mean: …`
- 인스턴스 존재 + `cascade = false` →
  `type '<name>' has <n> instance(s) (including subtypes). pass cascade => true to remove them`
  ([types.rs:698](../../engine/src/catalog/types.rs#L698))

---

## 4. 프로퍼티

### `og_add_property(graph text, type_name text, prop text, data_type text, required bool DEFAULT false, is_key bool DEFAULT false) RETURNS void`

정의: [engine/src/catalog/types.rs:510](../../engine/src/catalog/types.rs#L510) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 프로퍼티를 선언하고, 해당 타입과 **모든 서브타입**의 저장 테이블에 실제 컬럼을 추가한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 타입 이름 |
| `prop` | `text` | 필수 | — | 프로퍼티 이름(질의에서 쓰는 이름) |
| `data_type` | `text` | 필수 | — | 아래 표의 선언 타입 |
| `required` | `bool` | 선택 | `false` | `NOT NULL` 부여 |
| `is_key` | `bool` | 선택 | `false` | 고유 인덱스 부여 |

**지원하는 `data_type`** ([engine/src/catalog/types.rs:13](../../engine/src/catalog/types.rs#L13))

| 선언값(대소문자 무관) | 물리 컬럼 타입 |
|---|---|
| `string` \| `text` \| `str` | `text` |
| `int` \| `integer` \| `int4` | `int4` |
| `long` \| `bigint` \| `int8` | `int8` |
| `float` \| `double` \| `float8` | `float8` |
| `real` \| `float4` | `float4` |
| `bool` \| `boolean` | `bool` |
| `datetime` \| `timestamptz` \| `timestamp` | `timestamptz` |
| `date` | `date` |
| `uuid` | `uuid` |
| `numeric` \| `decimal` | `numeric` |
| `json` \| `jsonb` | `jsonb` |
| `text[]` \| `string[]` | `text[]` |
| `int[]` \| `bigint[]` | `int8[]` |
| `vector(N)` (N은 숫자) | `vector(N)` — pgvector |

그 외는 오류:
`unsupported property type '<decl>'. supported: string, int, long, float, bool, datetime, date, uuid, numeric, jsonb, text[], int[], vector(N)`
([types.rs:38](../../engine/src/catalog/types.rs#L38)).

**물리 컬럼 이름 규칙** ([engine/src/catalog/types.rs:53](../../engine/src/catalog/types.rs#L53)):
`p_` 접두사 + 소문자화, 영숫자·`_` 외의 문자는 `_`로 치환.
예: `beginTime` → `p_begintime`, `isbn-13` → `p_isbn_13`.

> ⚠️ 이 규칙은 대소문자를 지운다. `userName`과 `username`이 같은 컬럼
> `p_username`으로 충돌한다 → [12_improvements_api.md](12_improvements_api.md) API-04.

**부수 효과 (전부 확인됨)**

1. 서브타입 전체에 `ALTER TABLE ... ADD COLUMN IF NOT EXISTS`
   ([types.rs:550](../../engine/src/catalog/types.rs#L550)).
2. `required`면 `ALTER COLUMN ... SET NOT NULL`.
3. **선언 이전에 `__ext` jsonb에 들어가 있던 값들을 컬럼으로 이동**시키고 `__ext`에서 제거
   ([types.rs:561](../../engine/src/catalog/types.rs#L561)). "먼저 쓰고 나중에 인덱스"라는
   스키마리스 그래프의 통상적 순서를 지원하기 위한 것.
4. `is_key`면 `CREATE UNIQUE INDEX IF NOT EXISTS uq_<sub>_<col>`.
5. 서브타입의 별칭 뷰를 재생성 (뷰는 생성 시점 컬럼 목록을 고정하므로).
6. `og_catalog.schema_version` 증가.

**예제** ([examples/meeting-rooms/schema.sql:12](../../examples/meeting-rooms/schema.sql#L12))

```sql
SELECT og_add_property('meeting', 'MeetingRoom', 'name',     'string', true,  true);
SELECT og_add_property('meeting', 'MeetingRoom', 'seats',    'int',    false, false);
SELECT og_add_property('meeting', 'Reservation', 'begin_time', 'datetime', true, false);
-- named-argument form also works
SELECT og_add_property('default', 'Person', 'name', 'string', required => true);
```

**실패 조건**

| 입력 | 오류 |
|---|---|
| 그래프/타입 없음 | `graph '…' does not exist` / `type '…' does not exist. did you mean: …` |
| 알 수 없는 `data_type` | `unsupported property type '…'` |
| `required = true` 인데 기존 인스턴스 존재 | `cannot add required property '<prop>' to '<type>': <n> existing instance(s) would violate it. add it as optional, backfill, then tighten.` ([types.rs:532](../../engine/src/catalog/types.rs#L532)) |
| 같은 프로퍼티 재선언 | `property insert failed` — `og_catalog.property (type_id, name)` 유일 제약 위반이 패닉으로 표면화 ([types.rs:545](../../engine/src/catalog/types.rs#L545)) |
| `__ext` 값 이동 실패(타입 캐스트 불가) | `failed to move existing '<prop>' values into its column: <e>` ([types.rs:569](../../engine/src/catalog/types.rs#L569)) |

---

### `og_create_index(graph text, type_name text, prop text) RETURNS void`

정의: [engine/src/catalog/types.rs:603](../../engine/src/catalog/types.rs#L603) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 선언된 프로퍼티 컬럼에 B-tree 인덱스를 계층 전체에 만든다(spec 001 FR-016).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `type_name` | `text` | 필수 | — | 타입 이름 |
| `prop` | `text` | 필수 | — | 프로퍼티 이름 |

**반환**: 없음.

생성되는 인덱스 이름은 `ix_<sub_type_id>_<column_name>` 이며
`CREATE INDEX IF NOT EXISTS`이므로 재실행이 안전하다.

**예제**

```sql
SELECT og_create_index('default', 'Person', 'name');
```

**실패 조건**
- 그래프/타입 없음 → 위와 동일.
- **프로퍼티가 선언되어 있지 않아도 오류가 나지 않는다.** `column_name(prop)`은
  이름만 계산하므로 존재하지 않는 컬럼을 인덱싱하려다
  `index creation failed` 패닉이 난다([types.rs:611](../../engine/src/catalog/types.rs#L611)).
  Cypher `CREATE INDEX` 경로는 이 문제를 `ensure_property`로 미리 막지만
  ([engine/src/compat/ddl.rs:132](../../engine/src/compat/ddl.rs#L132)), SQL 함수 직접
  호출 경로에는 그 보호가 없다 → [12_improvements_api.md](12_improvements_api.md) API-06.

---

## 5. 역할(Role)과 규칙(Rule)

### `og_add_role(graph text, rel_type text, role text, player_type text, ordinal int4, card_min int4 DEFAULT 0, card_max int4 DEFAULT NULL) RETURNS void`

정의: [engine/src/catalog/types.rs:625](../../engine/src/catalog/types.rs#L625) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 관계 타입에 이름 붙은 참여 슬롯을 선언한다(spec 002 FR-004..FR-006). `ON CONFLICT (rel_type_id, name) DO UPDATE`이므로 재선언은 갱신이다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `rel_type` | `text` | 필수 | — | 관계 타입 이름 |
| `role` | `text` | 필수 | — | 역할 이름 |
| `player_type` | `text` | 필수(NULL 허용) | — | 이 역할을 맡을 수 있는 타입. `NULL`이면 제약 없음 |
| `ordinal` | `int4` | 필수 | — | `0` = 출발(src), `1` = 도착(dst), `2` 이상 = n-ary 참여자 |
| `card_min` | `int4` | 선택 | `0` | 최소 카디널리티 (카탈로그에 기록됨) |
| `card_max` | `int4` | 선택 | `NULL` | 최대 카디널리티 (카탈로그에 기록됨) |

**반환**: 없음.

**무엇을 강제하는가 (확인됨)**: `ordinal ∈ {0, 1}` 이고 `player_type_id IS NOT NULL`인
역할만 `og_create_edge` 쓰기 시점에 검사된다
([engine/src/storage/mod.rs:455](../../engine/src/storage/mod.rs#L455) `validate_roles`).
`card_min`/`card_max`는 **카탈로그에 저장만 되고 이 경로에서 강제되지 않는다** —
TypeQL 쓰기 경로는 별도로 카디널리티 상한을 검사한다
([engine/src/typeql/write.rs:278](../../engine/src/typeql/write.rs#L278)).

**예제** ([examples/demo.sql:38](../../examples/demo.sql#L38))

```sql
SELECT og_create_type('default', 'ACTED_IN', 'relation');
SELECT og_add_role('default', 'ACTED_IN', 'actor',      'Person', 0);
SELECT og_add_role('default', 'ACTED_IN', 'production', 'Work',   1);
```

이후 다음 쓰기는 거부된다:

```
ERROR:  role 'actor' of relation 'ACTED_IN' requires a 'Person', got 'Film'
```

**실패 조건**
- `rel_type`이 관계 타입이 아님 →
  `'<rel_type>' is not a relation type; roles only exist on relations`
  ([types.rs:638](../../engine/src/catalog/types.rs#L638))
- `player_type`이 존재하지 않음 → `type '…' does not exist. did you mean: …`

---

### `og_add_rule(graph text, rel_type text, characteristic text, target_type text DEFAULT NULL) RETURNS void`

정의: [engine/src/catalog/types.rs:657](../../engine/src/catalog/types.rs#L657) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 관계 특성을 선언한다(spec 002 FR-027).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `rel_type` | `text` | 필수 | — | 관계 타입 이름 |
| `characteristic` | `text` | 필수 | — | `transitive` / `symmetric` / `reflexive` / `inverse` (대소문자 무관) |
| `target_type` | `text` | 선택 | `NULL` | `inverse`일 때 필수 — 역관계가 되는 관계 타입 |

**반환**: 없음. `ON CONFLICT DO NOTHING`이므로 멱등.

**예제**

```sql
SELECT og_add_rule('default', 'COLLABORATED_WITH', 'symmetric');
SELECT og_add_rule('default', 'PART_OF', 'transitive');
SELECT og_add_rule('default', 'PARENT_OF', 'inverse', 'CHILD_OF');
```

**실패 조건**
- 목록 밖의 특성 →
  `unknown characteristic '<c>' (transitive|symmetric|reflexive|inverse)`
  ([types.rs:668](../../engine/src/catalog/types.rs#L668))
- `inverse`인데 `target_type`이 `NULL` →
  `characteristic 'inverse' requires a target relation type` ([types.rs:671](../../engine/src/catalog/types.rs#L671))

---

## 6. 상속 조회 — 구간(nested-set) 라벨

**결정(Decision)**: 서브타입 판정은 재귀 워크가 아니라 `og_catalog.type_label`의
`lft`/`rgt` 구간 비교 **한 번**이다
([engine/src/catalog/labeling.rs:192](../../engine/src/catalog/labeling.rs#L192)).
이 세 함수는 Cypher 컴파일러가 라벨 술어를 SQL로 펼칠 때 그대로 방출하는 것들이다.

### `og_subtypes(type_id int4) RETURNS int4[]`

정의: [engine/src/catalog/labeling.rs:192](../../engine/src/catalog/labeling.rs#L192) · 휘발성: `STABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

**무엇을 하는가**: 자기 자신과 모든 후손 타입 id를 반환한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `type_id` | `int4` | 필수 | — | 루트 타입 id. `NULL`이면 `STRICT`이므로 `NULL` 반환 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int4[]` | `NULL` 입력 시 NULL | 자기 자신 포함. 순서 보장 없음 |

**예제**

```sql
SELECT og_subtypes(og_type_id('default', 'Work'));
-- {12,13,14,15}  -- Work, Film, AnimatedFilm, Series
```

### `og_supertypes(type_id int4) RETURNS int4[]`

정의: [engine/src/catalog/labeling.rs:212](../../engine/src/catalog/labeling.rs#L212) · 휘발성: `STABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

**무엇을 하는가**: 자기 자신과 모든 조상 타입 id를 반환한다. Cypher `labels(n)`이 이 함수를 사용해 상위 체인 전체를 라벨 목록으로 돌려준다([engine/src/cypher/compile.rs:1475](../../engine/src/cypher/compile.rs#L1475)).

### `og_is_subtype(sub int4, sup int4) RETURNS bool`

정의: [engine/src/catalog/labeling.rs:232](../../engine/src/catalog/labeling.rs#L232) · 휘발성: `STABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

**무엇을 하는가**: `sub ⊑ sup` 판정. 존재하지 않는 id를 넣으면 `false`.

**예제**

```sql
SELECT og_is_subtype(og_type_id('default','AnimatedFilm'),
                     og_type_id('default','Work'));   -- t
```

### `og_relabel(graph_id int4) RETURNS void`

정의: [engine/src/catalog/labeling.rs:247](../../engine/src/catalog/labeling.rs#L247) · 휘발성: 기본값 · 병렬: `STRICT`

**무엇을 하는가**: 구간 라벨을 강제로 재계산한다. **진단·복구용**이며 정상 DDL 경로는 자동으로 호출한다.

**주의**: 인자가 그래프 **이름이 아니라 `graph_id`(int4)** 다 — 다른 DDL 함수와
유일하게 다른 관례 → [12_improvements_api.md](12_improvements_api.md) API-02.

```sql
SELECT og_relabel((SELECT graph_id FROM og_catalog.graph WHERE name = 'default'));
```

---

## 7. 인트로스펙션 뷰

```sql
SELECT * FROM og_type_view     WHERE graph = 'default';
SELECT * FROM og_property_view WHERE graph = 'default' AND type_name = 'Person';
SELECT * FROM og_role_view     WHERE graph = 'default';
```

컬럼 정의는 [00_index.md §4.10](00_index.md) 참조.

---

## 8. 금지 / 필수

- **필수**: 프로퍼티는 인덱스를 만들기 **전에** `og_add_property`로 선언할 것.
  `og_create_index`는 컬럼 존재를 확인하지 않는다.
- **필수**: `required => true`는 인스턴스가 **하나도 없을 때** 붙일 것. 이후에는
  "선택으로 추가 → 백필 → 조인다" 순서를 따를 것(오류 메시지가 이 순서를 안내한다).
- **금지**: `og_catalog.type` / `og_catalog.property`를 직접 수정하지 말 것.
  물리 컬럼과 구간 라벨이 함께 움직이지 않는다.
- **금지**: 추상 타입(`is_abstract => true`)에 인스턴스를 만들려 하지 말 것 —
  저장 테이블 자체가 없다(`'<name>' is abstract and cannot be instantiated`).
- **필수**: 프로퍼티 이름을 대소문자만 다르게 두지 말 것 — 물리 컬럼이 충돌한다(§4).

---

## 9. 관련 문서

- 데이터 쓰기 → [02_data_dml.md](02_data_dml.md)
- Neo4j `CREATE INDEX` / `CREATE CONSTRAINT` 경로 → [09_neo4j_compat.md](09_neo4j_compat.md)
- TypeQL `define` → [04_typeql.md](04_typeql.md)
- 오류 체계 → [11_errors.md](11_errors.md)

<!-- affects: api, backend, data -->
<!-- requires-update: 02_api/02_data_dml.md, 02_api/09_neo4j_compat.md -->
