# 쓰기 경로 — 왜 Rust인가, 한 트랜잭션에서 무엇이 잠금-스텝인가

> **이 문서가 답하는 질문**
> - 읽기는 SQL 한 문장인데 쓰기는 왜 Rust 절차 코드인가?
> - 엣지 하나를 만들 때 정확히 몇 개의 구조가 갱신되는가?
> - 미선언 프로퍼티가 실컬럼으로 승격되는 시점과 규칙은?
> - 타입이 충돌하면 무슨 일이 일어나는가?
> - 사용자 값은 어디서 SQL과 만나는가?

---

## 1. 결정 — 왜 쓰기만 Rust인가

`engine/src/storage/mod.rs:1-10`이 그대로 답이다.

> Write paths live here in Rust because they must keep three structures in
> lock-step inside one transaction (spec 001 FR-012): the registry, the typed
> property table, and both directions of the adjacency segment.
>
> Read paths are deliberately **not** here.

즉:

- **쓰기**: 서로 다른 3개 구조를 한 트랜잭션 안에서 함께 움직여야 한다.
  이건 SQL 한 문장으로 표현할 수 없고, 표현하려면 트리거를 써야 하는데
  트리거는 순서와 재진입이 눈에 보이지 않는다.
- **읽기**: Rust 함수로 감싸면 **옵티마이저 장벽**이 된다.
  플래너가 순회 자체를 못 보게 되므로, 이 프로젝트가 Apache AGE와 다르다고 주장하는
  근거 자체가 사라진다.

**따라서 이것은 "성능이 필요해서 Rust"가 아니라 "정합성이 필요해서 Rust, 성능이 필요해서 SQL"이다.**

---

## 2. 사실 — 잠금-스텝으로 유지되는 3구조

### 2.1 노드 생성 — 2구조

`engine/src/storage/mod.rs:253-291` (`create_node_inner`):

| 순서 | 구조 | 문장 | 라인 |
|---|---|---|---|
| 0 | 식별자 발급 | `INSERT INTO og_data.og_id_alloc ... ON CONFLICT DO UPDATE ... RETURNING next_id - 1` | `mod.rs:25-32` |
| 1 | **레지스트리** | `INSERT INTO og_data.og_node (id, type_id) VALUES ($1, $2)` | `mod.rs:271-275` |
| 2 | **타입별 프로퍼티 테이블** | `INSERT INTO og_data.n_<tid> (id, p_..., __ext) VALUES ($1, ..., ...)` | `mod.rs:277-286` |

노드는 인접 세그먼트를 만들지 않는다. 세그먼트는 엣지가 생길 때 생긴다.

`alloc_id`는 `og_id_alloc`에 대한 UPSERT이므로 같은 타입에 대한 동시 삽입은 **그 행에서 직렬화**된다
(`mod.rs:24-34`). 이것이 사실상 타입 단위의 쓰기 직렬화 지점이다.

### 2.2 엣지 생성 — 4구조

`engine/src/storage/mod.rs:402-452` (`create_edge_inner`):

| 순서 | 구조 | 문장 | 라인 |
|---|---|---|---|
| 0 | 롤 검증 | `SELECT name, ordinal, player_type_id FROM og_catalog.role ...` → `og_is_subtype` | `mod.rs:416`, `455-484` |
| 1 | 식별자 발급 | `og_id_alloc` UPSERT | `mod.rs:418` |
| 2 | **레지스트리** | `INSERT INTO og_data.og_edge (id, type_id, src, dst)` | `mod.rs:429-433` |
| 3 | **타입별 프로퍼티 테이블** | `INSERT INTO og_data.e_<tid> (id, src, dst, p_..., __ext)` | `mod.rs:435-442` |
| 4 | **인접 세그먼트 — 정방향** | `adjacency::append(src, tid, 'o', dst, eid)` | `mod.rs:445` |
| 5 | **인접 세그먼트 — 역방향** | `adjacency::append(dst, tid, 'i', src, eid)` | `mod.rs:446` |

`mod.rs:444`의 주석이 명시한다: `// Both adjacency directions, same transaction — spec 001 FR-012.`

**역방향을 함께 쓰는 이유**: `MATCH (a)<-[:R]-(b)`가 `b`의 정방향 세그먼트를 스캔하는 대신
`a`의 역방향 세그먼트 한 튜플만 읽으면 되게 하려는 것이다. 저장 비용은 정확히 2배이고,
그 대가로 양방향 확장이 대칭적으로 상수 시간이다.

### 2.3 삭제 — 같은 순서를 역으로

`delete_edge_inner` (`mod.rs:501-528`):

```
og_edge에서 (src, dst) 조회 → 없으면 0 반환
adjacency::remove(src, tid, 'o', eid)
adjacency::remove(dst, tid, 'i', eid)
DELETE FROM og_data.e_<tid> WHERE id = $1
DELETE FROM og_data.og_edge WHERE id = $1
DELETE FROM og_data.og_role_player WHERE edge_id = $1
```

`delete_node_inner` (`mod.rs:355-383`)는 먼저 입사 엣지를 전부 모은다:

```sql
SELECT DISTINCT e FROM og_data.og_adj a, LATERAL unnest(a.eid) AS e
 WHERE a.src = $1
```

`dir`을 걸지 않았으므로 `'o'`와 `'i'` 세그먼트가 모두 잡히고,
따라서 **나가는 엣지와 들어오는 엣지가 함께** 수집된다. 이것이 `DETACH DELETE` 의미론이다.

---

## 3. 사실 — 인접 세그먼트의 물리 구조

`engine/sql/bootstrap.sql:200-217`:

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

- `CHUNK = 256` (`storage/adjacency.rs:15`). 256 × 8B × 2배열 = 4 KB → 8 KB 힙 페이지 안에 들어간다.
- `STORAGE MAIN`이 **핵심**이다. TOAST로 빠지면 방금 산 지역성이 사라진다 (`bootstrap.sql:210-211`).
- `(etype, dir)`로 나뉘어 있으므로 관계 타입/방향 가지치기가 인덱스 없이 공짜다.
- `seq`로 나뉘어 있으므로 슈퍼노드도 한 번에 물질화하지 않고 스트리밍된다.

### 3.1 append — 꼬리 세그먼트 read-modify-write

`storage/adjacency.rs:19-44`:

```sql
UPDATE og_data.og_adj a
   SET nbr = a.nbr || $4::int8, eid = a.eid || $5::int8, n = a.n + 1
 WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::"char"
   AND a.seq = (SELECT max(seq) FROM og_data.og_adj
                 WHERE src = $1 AND etype = $2 AND dir = $3::text::"char")
   AND a.n < $6
 RETURNING a.seq
```

`RETURNING`이 비면(= 꼬리가 꽉 찼거나 세그먼트가 없으면) 새 청크를 `INSERT`한다
(`adjacency.rs:33-43`). `seq`는 `COALESCE((SELECT max(seq)+1 ...), 0)`으로 계산한다.

> **동시성 주의**: 두 트랜잭션이 같은 `(src, etype, dir)`에 동시에 append하고
> 둘 다 UPDATE에서 0행을 받으면, 둘 다 같은 `seq`를 계산해 INSERT하므로
> 기본키 충돌이 난다. 자세한 내용은 [`07_transactions_and_concurrency.md`](07_transactions_and_concurrency.md).

### 3.2 remove — 같은 인덱스에서 두 배열을 함께 자른다

`storage/adjacency.rs:48-72`. `array_position(eid, $4)`로 위치를 찾고
`nbr`과 `eid`를 **같은 인덱스로** 스플라이스한다. 두 배열의 정렬이 깨지면
`unnest(nbr, eid)`가 잘못된 쌍을 만들어내므로 이것이 불변식이다.
비워진 세그먼트는 `n = 0` 조건으로 즉시 회수한다 (`adjacency.rs:66-71`).

이 불변식은 `og_check_integrity()`가 검사한다 (`storage/stats.rs`, `segment_length_mismatch`).

---

## 4. 사실 — 프로퍼티 승격 (`declare_new_props`)

Cypher 애플리케이션은 아무것도 선언하지 않는다. Neo4j에는 선언할 스키마가 없기 때문이다.
그래서 **쓰기 시점에** 새 프로퍼티를 실컬럼으로 승격시킨다.

### 4.1 승격 판정

`storage/mod.rs:53-60` — JSON 값에서 컬럼 타입을 추론한다:

| JSON | 컬럼 타입 |
|---|---|
| `true` / `false` | `bool` |
| 정수 | `int8` |
| 실수 | `float8` |
| 문자열 | `text` |
| 배열 / 객체 | **승격 안 함** → `__ext` jsonb에 남는다 |

배열을 승격하지 않는 이유는 주석에 명시돼 있다 (`mod.rs:49-52`):
유일하게 중요한 배열 프로퍼티인 `embedding`은 `og_add_embedding`이
`vector(N)`으로 선언하는데, 먼저 jsonb로 선언해버리면 그 길이 막힌다.

### 4.2 승격 실행

`storage/mod.rs:87-158`:

```
1. type_id → (graph 이름, 타입 이름) 조회 (mod.rs:90-103)
2. props의 각 키에 대해:
   a. 컬럼 타입 추론 실패 → 건너뜀
   b. 기존 프로퍼티에 없음  → og_add_property(graph, type, key, want, false, false)
   c. 있고 타입이 안 맞음   → 아래 4.3 (widening)
```

`og_add_property`(`catalog/types.rs:511-600`)는 그냥 컬럼만 추가하는 게 아니다:

- `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` — 모든 서브타입 테이블에 (`types.rs:550`)
- **`__ext`에 이미 있던 값을 컬럼으로 끌어올린다** (`types.rs:561-570`):
  ```sql
  UPDATE <table> SET <col> = (__ext ->> '<prop>')::<dtype>, __ext = __ext - '<prop>'
   WHERE __ext ? '<prop>'
  ```
  이것이 없으면 "먼저 쓰고 나중에 인덱스" 순서에서 인덱스가 빈 컬럼을 보게 된다.
- 별칭 뷰 재생성 (`types.rs:594-598`) — 뷰는 생성 시점에 컬럼 목록을 고정하므로
- `bump_schema_version()` (`types.rs:599`) → **생성된 타입 뷰 전체 폐기** (`labeling.rs:175`)

### 4.3 타입 충돌 — 단방향 text 확장

`storage/mod.rs:62-73`:

```rust
const WIDENABLE: &[&str] = &["bool", "int8", "float8"];

fn type_accepts(declared: &str, wanted: &str) -> bool {
    if declared == wanted { return true; }
    matches!((declared, wanted),
             ("float8", "int8") | ("numeric", "int8") | ("numeric", "float8"))
}
```

**규칙**: 추론으로 만들었을 수 있는 타입(`bool`/`int8`/`float8`)만 확장 대상이다.
`vector(1536)`이나 `timestamptz`처럼 **의도적으로 선언된** 타입은 건드리지 않는다.
`mod.rs:121-126` 주석이 그 이유를 기록해 두었다 —
2026-08-16에 그렇게 해서 벡터 스위트를 깨뜨린 적이 있다.

확장 시 하는 일 (`mod.rs:127-153`):

```
모든 서브타입에 대해:
  drop_alias_view(name)                          -- 뷰가 컬럼에 의존하므로 먼저 제거
  ALTER TABLE <t> ALTER COLUMN <c> TYPE text USING <c>::text
  ensure_alias_view(sub, name, table)
UPDATE og_catalog.property SET data_type = 'text' WHERE type_id = ANY(...) AND name = $2
```

> **알려진 엣지 케이스**: `int8` 컬럼에 실수를 쓰면 `type_accepts("int8","float8") == false`이고
> `int8`은 `WIDENABLE`이므로 **`float8`이 아니라 `text`로 확장된다.**
> 즉 `SET n.score = 1` 다음에 `SET n.score = 1.5`를 하면 숫자 비교와 인덱스가 사라진다.
> → [`11_improvements_code.md`](11_improvements_code.md) `CODE-06`.

> **알려진 위험**: 위 두 `Spi::run`은 `let _ =`로 결과를 버린다 (`mod.rs:138`, `mod.rs:147`).
> `ALTER TABLE`이 실패하고 카탈로그 UPDATE가 성공하면 **카탈로그는 text, 컬럼은 int8**이 된다.
> → `CODE-07`.

---

## 5. 사실 — 사용자 값은 어디서 SQL과 만나는가

이것이 이 코드베이스에서 가장 엄격하게 지켜지는 규칙이다 (spec 003 FR-026).

### 5.1 Cypher 읽기 — 파라미터 하나

`cypher/compile.rs:18`:

```rust
pub const PARAM: &str = "$1";
```

`$param`은 항상 `($1 ->> 'name')` 형태로 컴파일된다 (`compile.rs:1156-1162`).
실행 시 `exec_json`이 jsonb **한 개**를 바인딩한다 (`cypher/mod.rs:148`):

```rust
client.select(sql, None, &[JsonB(params.clone()).into()])
```

즉 사용자 파라미터는 SQL 텍스트에 **한 글자도 들어가지 않는다.**

### 5.2 Cypher 쓰기 — 프로퍼티 페이로드 하나

`plan_props`(`storage/mod.rs:160-222`)는 컬럼 목록과 **값 표현식**을 만든다.
값 표현식은 리터럴이 아니라 바인딩 파라미터에서 꺼내는 식이다 (`mod.rs:209-217`):

```rust
// scalar
format!("({param}->>{lit})::{dtype}")
// array
format!("(SELECT array_agg(x)::{dtype} FROM jsonb_array_elements_text({param}->{lit}) AS t(x_raw), \
          LATERAL (SELECT t.x_raw::{elem}) AS c(x))")
```

여기서 `{param}`은 `"$2"`(노드) 또는 `"$4"`(엣지)이고, `{lit}`은 **프로퍼티 이름**이다.
프로퍼티 이름은 값이 아니라 카탈로그에서 온 선언된 이름이며,
`quote_json_key`(`mod.rs:224-226`)로 작은따옴표를 이스케이프한다.

실제 실행 (`mod.rs:285`):

```rust
Spi::run_with_args(&sql, &[nid.into(), JsonB(props).into()])
```

**프로퍼티 값 전체가 jsonb 파라미터 하나다.**

### 5.3 문자열로 조립되는 것들 — 그리고 그 이유

| 조립되는 것 | 근거 라인 | 안전한 이유 |
|---|---|---|
| 테이블 이름 `og_data.n_<tid>` | `catalog/types.rs:68-74` | `tid`는 `int4` |
| 컬럼 이름 `p_<prop>` | `catalog/types.rs:53-66` | 영숫자/`_` 외는 `_`로 치환. 다만 비-ASCII 문자는 보존 |
| 타입 ID 배열 `ARRAY[1,2,3]::int4[]` | `cypher/compile.rs:844-847` | 전부 `i32` |
| SQL 문자열 리터럴 | `cypher/compile.rs:1589-1591` `sql_str` | `'` → `''` 이스케이프 |
| SQL 식별자 | `cypher/compile.rs:1584-1586` `quote_ident` | `"` → `""` 이스케이프 + 항상 인용 |

`column_name`(`types.rs:53-66`)의 비-ASCII 보존은 의도적이다 — 한국어 프로퍼티 이름
`이름`과 `용량`을 전부 `_`로 접으면 서로 다른 프로퍼티가 같은 컬럼으로 합쳐진다.

> **예외 — TypeQL 쓰기 경로**: `typeql/write.rs:649-674` `typed_literal`은 속성 값을
> **SQL 리터럴 텍스트로** 만들어 문장에 끼워 넣는다 (`write.rs:242`, `write.rs:257`, `write.rs:321`).
> 이스케이프는 `lit_str`(`typeql/compile.rs:616-618`)로 하고 있으나,
> Cypher 경로가 지키는 "바인딩 파라미터 한 개" 규칙과는 다르다.
> → `CODE-08`.

---

## 6. 사실 — 쓰기 통계 (Neo4j `ResultSummary.counters`)

`engine/src/stats.rs`. 백엔드-로컬 `thread_local` 카운터이며, `og_cypher()` 진입 시마다
`reset()` 된다 (`cypher/mod.rs:92`).

`stats.rs:9-13`이 그 한계를 명시한다:

> The state is per-backend and reset at the start of every `og_cypher()` call. …
> It is *not* a transaction log — a rolled-back statement leaves its counts behind,
> and the next call clears them.

카운트 지점은 실제 변경이 일어나는 `storage` 안이다:
`node_created()`(`storage/mod.rs:287`), `relationship_created()`(`mod.rs:448`),
`properties_set()`(`mod.rs:288,449` 등), `node_deleted()`(`mod.rs:381`),
`relationship_deleted()`(`mod.rs:526`).

인덱스/제약 카운터는 `compat/ddl.rs:40-45`에서 올린다.

Bolt 게이트웨이는 쓰기 직후 같은 커넥션에서 `og_cypher_stats()`를 한 번 읽고
summary에 실어 보낸다 (`bolt/src/session.rs:373-377, 430-436`).

---

## 7. 결정 요약

| 결정 | 근거 | 대가 |
|---|---|---|
| 쓰기만 Rust 절차 코드 | `storage/mod.rs:1-10` | 배치 쓰기가 행당 여러 번의 SPI 왕복 (`CODE-09`) |
| 양방향 인접 세그먼트 | `storage/mod.rs:444-446` | 인접 저장 공간 2배 |
| `CHUNK = 256`, `STORAGE MAIN` | `bootstrap.sql:200-211` | 슈퍼노드는 여러 세그먼트로 쪼개짐 → `og_reorganize()` 필요 |
| 쓰기 시점 프로퍼티 승격 | `storage/mod.rs:78-86` | 쓰기 경로에서 DDL이 발생 (`CODE-05`) |
| 타입 충돌 시 단방향 text 확장 | `storage/mod.rs:83-86` | int→float 승격이 text로 떨어짐 (`CODE-06`) |
| 사용자 값은 jsonb 파라미터 1개 | spec 003 FR-026 | TypeQL 쓰기 경로는 아직 미준수 (`CODE-08`) |

---

## 금지 / 필수

- **금지**: 읽기 경로를 `#[pg_extern]` Rust 함수로 감싸는 것. 옵티마이저 장벽이 된다.
  (예외: `og_reach` — 방문집합이 필요하고, 그 대가로 `access.sql:192-197`에서 `ROWS 100`을 맞춰 준다.)
- **금지**: 사용자가 제공한 **값**을 SQL 텍스트에 `format!`으로 넣는 것.
  반드시 `Spi::run_with_args` / `client.select(..., &[...])`의 바인딩 파라미터로 넘긴다.
- **금지**: 인접 세그먼트의 `nbr`과 `eid`를 서로 다른 인덱스로 조작하는 것.
- **필수**: 엣지를 만들거나 지울 때 **양방향 세그먼트를 같은 함수 안에서** 함께 갱신한다.
  한쪽만 갱신하면 `og_check_integrity()`가 `missing_adjacency`로 잡는다.
- **필수**: 새 스토리지 테이블을 만들면 `og_catalog.type.storage_table`에 기록한다.
  기록하지 않으면 `views::concrete_tables`가 그 타입을 보지 못한다 (`cypher/views.rs:65-70`).

<!-- affects: backend, data -->
<!-- requires-update: 06_data/, 02_api/ -->
