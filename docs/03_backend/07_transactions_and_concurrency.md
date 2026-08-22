# 트랜잭션 경계, 락, SPI 재진입, 동시성 정책

> **이 문서가 답하는 질문**
> - 트랜잭션 경계는 정확히 어디인가?
> - 명시적 락은 어디에 있는가? (답: 없다)
> - SPI 재진입은 어떻게 다루는가?
> - 어떤 상태가 트랜잭션 바깥에 있는가?
> - 무엇이 동시성에 안전하지 않은가?

---

## 1. 사실 — 트랜잭션 경계는 호출자의 것이다

이 확장은 **트랜잭션을 시작하지도 끝내지도 않는다.**
`BEGIN` / `COMMIT` / `ROLLBACK` / `SAVEPOINT`를 실행하는 Rust 코드는 없다
(전 소스 grep 결과 0건).

| 진입점 | 트랜잭션 경계 |
|---|---|
| `psql`에서 `SELECT og_cypher(...)` | PostgreSQL의 암묵적 트랜잭션 1개 |
| `BEGIN; SELECT og_cypher(...); SELECT og_cypher(...); COMMIT;` | 사용자가 연 것 하나 |
| Bolt 자동 커밋 `RUN` | 암묵적 트랜잭션 1개 |
| Bolt 명시적 `BEGIN` … `COMMIT` | `bolt/src/session.rs:208-219`가 `batch_execute("BEGIN")` 등으로 전달 |
| `og_typeql_script(graph, script)` | **스크립트 전체가 한 트랜잭션** (`typeql/mod.rs:98-113`) |

따라서 실패는 항상 전부 아니면 전무다 — 단, 3절의 백엔드-로컬 상태는 예외다.

`og_typeql_script`는 블록 i에서 실패하면 `error!("typeql error in block {} of {}")`를 내고
(`typeql/mod.rs:107-111`), 그 오류가 트랜잭션 전체를 abort시킨다.

---

## 2. 사실 — SPI 사용 규약

`engine/src/spiu.rs`가 세 개의 헬퍼만 노출한다:

| 함수 | 내부 | 용도 |
|---|---|---|
| `one::<T>(sql, args)` | `Spi::connect` + `client.select` | 읽기 전용 단일 값 |
| `two::<A,B>(sql, args)` | `Spi::connect` + `client.select` | 읽기 전용 두 값 |
| `one_mut::<T>(sql, args)` | `Spi::connect_mut` + `client.update` | `INSERT … RETURNING` 등 |

존재 이유 (`spiu.rs:3-6`):

> `Spi::get_one_with_args` raises `InvalidPosition` when a query returns no rows,
> which conflates "nothing matched" with "something broke".

세 함수 모두 빈 결과에 대해 `Ok(None)`을 돌려준다. "타입이 없다"는 정상적이고 보고 가능한 상황이지
내부 오류가 아니다(spec 008 FR-008).

### 2.1 SPI 재진입

이 코드베이스에서 SPI는 **중첩된다.** 예:

```
og_cypher()                                 [SPI 레벨 0 — 함수 자체가 SQL 안에 있다]
  └ Spi::connect(|client| client.select(compiled_sql, ...))     [레벨 1]
      └ compiled_sql 안의 og_node_json(id)                       [레벨 2, plpgsql]
          └ 그 안의 EXECUTE format(...)                          [레벨 3]
```

쓰기 경로는 더 깊다:

```
og_cypher()
  └ exec_json(select_sql)          바인딩 행 생성          [레벨 1]
  └ 행마다:
      create_node_inner()
        └ alloc_id → Spi::connect_mut                     [레벨 1]
        └ plan_props → Spi::connect × 1~2                 [레벨 1]
            └ declare_new_props → Spi::run_with_args("SELECT og_add_property(...)")
                └ og_add_property 안에서 ALTER TABLE / UPDATE / CREATE VIEW  [레벨 2]
                    └ bump_schema_version → drop_all_views → DROP VIEW ...  [레벨 3]
        └ Spi::run_with_args(INSERT ...)                  [레벨 1]
```

**규약**: 모든 SPI 호출은 `Spi::connect` / `Spi::connect_mut` 클로저 안에서 끝난다.
클로저 밖으로 `SpiClient`나 `SpiTupleTable`을 반출하지 않는다.
결과는 항상 `Vec`이나 스칼라로 **물질화한 뒤** 반환한다
(예: `cypher/views.rs:36-52`, `storage/mod.rs:161-177`, `storage/traverse.rs:107-158`).

### 2.2 준비된 문장의 재사용

`og_reach`(`storage/traverse.rs:107-116`)만 예외적으로 `client.prepare()`를 쓴다:

```rust
let stmt = client.prepare(sql.as_str(), &[INT8ARRAYOID, INT4ARRAYOID])?;
```

이유는 주석에 있다 (`traverse.rs:100-106`):

> On a graph with a large diameter the frontier can be a single node and the level
> count can be six figures — a 1,000,000-node chain walked to 100,000 hops does almost
> no work per level and nothing but levels. Connecting and re-planning inside the loop
> made that case ten times slower than the same walk written as a plain recursive CTE.

---

## 3. 사실 — 트랜잭션 바깥에 있는 상태

세 개의 `thread_local!` 상태가 있다. 전부 **백엔드-로컬이고 롤백되지 않는다.**

| 상태 | 위치 | 수명 | 롤백 시 |
|---|---|---|---|
| `PLAN_CACHE` | `cypher/mod.rs:26-31` | 백엔드 | 남는다 |
| `CSR` (컴파일된 그래프) | `storage/traverse.rs:205-210` | 백엔드 | 남는다 |
| 쓰기 카운터 | `stats.rs:23-25` | `og_cypher()` 호출 1회 | 남는다 |

### 3.1 `PLAN_CACHE` — 무효화가 없다

`cypher/mod.rs:47-67`:

```rust
let key = (graph.to_string(), query.to_string());
if let Some(hit) = PLAN_CACHE.with(|c| c.borrow().get(&key).cloned()) { return Ok(hit); }
...
PLAN_CACHE.with(|cache| {
    let mut m = cache.borrow_mut();
    if m.len() > 512 { m.clear(); }
    m.insert(key, out.clone());
});
```

키는 `(graph, query)`뿐이다. **스키마 버전이 키에 없다.**

그런데 컴파일 산출물은 스키마에 강하게 의존한다:

- 해석된 타입 id (`ARRAY[7]::int4[]`)
- 생성된 타입 뷰 이름 (`og_data.v_5`)
- 라벨이 없어서 `false`로 굳은 술어

그리고 스키마가 바뀌면 `bump_schema_version()`이 `drop_all_views()`를 부른다
(`catalog/labeling.rs:172-182`) — **뷰가 전부 지워진다.**

같은 세션에서:

```sql
SELECT og_cypher('g', 'MATCH (p:Person) RETURN p');   -- v_5 를 참조하는 SQL이 캐시됨
SELECT og_cypher('g', 'CREATE (:NewLabel {a:1})');    -- 새 타입 → relabel → drop_all_views
SELECT og_cypher('g', 'MATCH (p:Person) RETURN p');   -- 캐시 히트 → v_5 는 이미 없음
```

→ `CODE-01`. 자세한 내용은 [`11_improvements_code.md`](11_improvements_code.md).

### 3.2 `CSR` — 스냅샷이 얼어 있다

`storage/traverse.rs:19-23`:

> `og_csr_build` / `og_csr_reach` — the pgGraph bet. Compile the topology once into a
> backend-local CSR of dense `u32` indices and walk it with no SPI, no heap and no
> planner in the loop. Faster, and it gives up exactly what leaving the heap gives up:
> **the snapshot is frozen at build time and RLS is never consulted.**

`traverse.rs:206-208`이 Rust 힙에 두는 이유도 명시한다: PostgreSQL 메모리 컨텍스트에 두면
트랜잭션 끝에 해제되는데, 다음 문장이 이미 만들어진 것을 찾는 게 요점이기 때문이다.

**운영상 함의**:

- `og_csr_build()` 이후의 쓰기는 CSR에 반영되지 않는다. 다시 빌드해야 한다.
- RLS가 걸린 그래프에서 `og_csr_reach()`는 **정책을 우회한다.**
- 커넥션 풀 뒤에서는 어느 백엔드가 CSR을 갖고 있는지 알 수 없다.
  `og_csr_reach()`는 없으면 `error!("no compiled graph in this backend — call og_csr_build() first")`
  (`traverse.rs:339-341`).

### 3.3 쓰기 카운터

`stats.rs:9-13`이 이미 정직하게 적어 두었다:

> It is *not* a transaction log — a rolled-back statement leaves its counts behind,
> and the next call clears them.

`og_cypher_stats()`가 `volatile, parallel_unsafe`로 선언된 이유도 이것이다 (`cypher/mod.rs:117`).

---

## 4. 사실 — 명시적 락이 없다

`SELECT … FOR UPDATE`, `LOCK TABLE`, `pg_advisory_lock`, `SET TRANSACTION ISOLATION`
— **전부 0건**이다. 따라서 동시성 정책은 전적으로 PostgreSQL 기본값
(Read Committed + 행 수준 락 + 고유 인덱스)에 의존한다.

### 4.1 사실상의 직렬화 지점 — `og_id_alloc`

`storage/mod.rs:24-34` `alloc_id`:

```sql
INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 2)
ON CONFLICT (type_id) DO UPDATE SET next_id = og_id_alloc.next_id + 1
RETURNING next_id - 1
```

같은 `type_id`에 대한 두 트랜잭션은 이 행에서 **직렬화된다.** 즉 한 타입에 대한
동시 삽입은 커밋될 때까지 서로를 막는다. 정합성 측면에서는 좋고, 처리량 측면에서는 병목이다.

### 4.2 위험 — 인접 세그먼트 append의 경합

`storage/adjacency.rs:19-44`:

```
UPDATE og_adj SET nbr = nbr || $4, ... WHERE ... AND seq = (SELECT max(seq) ...) AND n < 256
RETURNING seq
-- 0행이면:
INSERT INTO og_adj (..., seq, ...) VALUES (..., COALESCE((SELECT max(seq)+1 ...), 0), ...)
```

시나리오:

- **꼬리가 있고 안 찼을 때**: UPDATE가 행 락을 잡으므로 T2는 T1 커밋까지 대기 → 안전.
- **세그먼트가 아예 없을 때**: T1, T2 둘 다 UPDATE에서 0행 → 둘 다 `seq = 0`으로 INSERT →
  둘 중 하나가 기본키 `(src, etype, dir, seq)` 위반으로 실패한다.
- **꼬리가 꽉 찼을 때**: 둘 다 `max(seq)+1`을 같은 값으로 계산 → 같은 충돌.

결과는 데이터 손상이 아니라 **오류**다(트랜잭션이 abort되므로). 하지만
"같은 노드에 동시에 엣지를 붙이면 간헐적으로 실패한다"는 운영상 놀라움이다. → `CODE-23`.

`ON CONFLICT`가 없다는 점에 주목: `adjacency.rs:34-43`의 INSERT에는 충돌 처리가 없다.

### 4.3 위험 — `MERGE`는 원자적이 아니다

`cypher/mod.rs:451-504` `merge_pattern`:

```
1. 패턴을 컴파일하고 이미 바인딩된 변수를 고정점으로 핀 고정   461-480
2. LIMIT 1 SELECT 실행                                        490-491
3. 결과가 있으면 → env 갱신 + ON MATCH SET, 반환               492-499
4. 없으면 → create_pattern + ON CREATE SET                     502-503
```

2와 4 사이에 락이 없다. 두 세션이 같은 `MERGE`를 동시에 실행하면 둘 다 만든다.

**유일한 방어선은 고유 인덱스**다. Cypher 애플리케이션은 보통
`CREATE CONSTRAINT … REQUIRE n.x IS UNIQUE`로 이를 만든다 (`compat/ddl.rs:284-325` →
`enforce_unique` `ddl.rs:156-167`). 제약을 만들지 않으면 중복이 생긴다.

`compat/ddl.rs:322-323`이 이 결정을 명시한다:

> Uniqueness is enforced, because that half is checkable at write time and is what
> callers rely on for MERGE to be idempotent.

### 4.4 위험 — TypeQL `put`도 원자적이 아니다

`typeql/write.rs:112-120`. `find_one` → 없으면 `run_insert`. 같은 구조.

속성은 `og_data.a_<tid>.val`의 `UNIQUE`가 막아 준다 (`typeql/schema.rs:273-276`).
엔티티/관계 인스턴스에는 그런 인덱스가 없다.

### 4.5 위험 — `intern_attribute`의 검사-후-삽입

`typeql/write.rs:241-261`:

```
SELECT id FROM <table> WHERE val = <lit>   → 있으면 재사용
없으면 alloc_id + INSERT
```

동시 실행 시 둘 다 "없음"을 보면 둘 다 INSERT하고, `val`의 `UNIQUE`가 하나를 거절한다.
`ON CONFLICT DO NOTHING`이나 재시도가 없으므로 오류가 사용자에게 그대로 나간다.

`@key` / `@unique` 검사(`write.rs:314-336`)도 마찬가지 — 애플리케이션 레벨 검사이므로
동시성 하에서는 놓칠 수 있다.

---

## 5. 사실 — 쓰기 경로 안의 DDL

이것이 이 코드베이스에서 가장 무거운 동시성 위험이다.

평범한 `CREATE (:Person {age: 30})` 하나가 다음을 유발할 수 있다:

| 트리거 | DDL | 락 |
|---|---|---|
| 새 라벨 | `create_type_inner` → `CREATE TABLE og_data.n_<tid>` + `CREATE VIEW` | 새 객체 |
| 새 라벨 | `relabel_graph` → `DELETE`/`INSERT` on `og_catalog.type_label` (그래프 전체) | 행 락 다수 |
| 새 라벨 | `bump_schema_version` → `drop_all_views` → `DROP VIEW ... CASCADE` × N | **AccessExclusive** on 각 뷰 |
| 새 프로퍼티 | `og_add_property` → `ALTER TABLE … ADD COLUMN` (모든 서브타입) | AccessExclusive, 단 fast default라 rewrite 없음 |
| 새 프로퍼티 | `UPDATE <table> SET <col> = (__ext->>'x')::t WHERE __ext ? 'x'` | **전체 테이블 UPDATE** |
| 타입 충돌 | `ALTER TABLE … ALTER COLUMN … TYPE text` | **AccessExclusive + 전체 재작성** |

근거: `catalog/types.rs:411-452`, `types.rs:539-599`, `catalog/labeling.rs:117-182`,
`storage/mod.rs:127-153`.

`relabel_graph`(`labeling.rs:117-170`)는 **그래프의 모든 라벨을 지우고 다시 넣는다.**
`DELETE FROM og_catalog.type_label WHERE graph_id = $1` 후 행마다 개별 `INSERT`
(`labeling.rs:154-167`) — 타입이 1,000개면 SPI 왕복 1,000회다.

**함의**:

- 스키마가 안정되기 전(초기 적재)에는 쓰기 처리량이 낮고 락 경합이 크다.
- 두 세션이 동시에 새 라벨을 쓰면 `relabel_graph`가 서로를 기다린다.
- 이 DDL들은 **사용자 트랜잭션 안에서** 일어나므로, 롤백하면 되돌아간다.
  하지만 그 사이 잡은 AccessExclusive 락은 커밋/롤백까지 유지된다.

권장: **스키마를 먼저 선언하고 적재한다.** `og_create_type` / `og_add_property` /
TypeQL `define`을 먼저 돌리면 쓰기 경로가 DDL을 유발하지 않는다.

---

## 6. 사실 — 병렬 실행 라벨

pgrx의 `#[pg_extern]` 속성이 PostgreSQL 함수 속성이 된다.

| 함수 | 라벨 | 근거 |
|---|---|---|
| `og_expand`, `og_vlp`, `og_reach_sql`, `og_subtype_ids`, `og_type_name` 등 | `STABLE PARALLEL SAFE` | `access.sql` 전반 |
| `og_node_json`, `og_edge_json` | `STABLE` (plpgsql, **PARALLEL 미지정 = UNSAFE**) | `access.sql:209,238` |
| `og_subtypes`, `og_supertypes`, `og_is_subtype` | `stable, parallel_safe, strict` | `catalog/labeling.rs:192,212,232` |
| `og_degree`, `og_degree_all` | `stable, parallel_safe, strict` | `storage/adjacency.rs:76,88` |
| `og_id_*`, `og_make_id` | `immutable, parallel_safe, strict` | `id.rs:73-91` |
| `og_reach`, `og_csr_reach`, `og_csr_hops` | `stable, parallel_restricted` | `traverse.rs:80,359,442` |
| `og_cypher_check`, `og_cypher_columns` | `immutable, parallel_safe` | `cypher/mod.rs:699,717` |
| `og_cypher_sql`, `og_typeql_sql` | `stable` | `cypher/mod.rs:74`, `typeql/mod.rs:82` |
| `og_cypher`, `og_typeql` | (기본) `VOLATILE` | `cypher/mod.rs:83`, `typeql/mod.rs:48` |
| `og_cypher_stats` | `volatile, parallel_unsafe` | `cypher/mod.rs:117` |

**`og_node_json` / `og_edge_json`이 `PARALLEL SAFE`가 아니라는 점이 중요하다.**
컴파일러가 타입 미상 프로퍼티 접근에 이 함수를 쓰므로(`compile.rs:991,1013`),
`MATCH (n) WHERE n.x = 1`류의 질의는 **병렬 계획을 아예 못 받는다.**
라벨이 붙은 질의는 실컬럼을 쓰므로 병렬 가능하다. → `CODE-24`.

### 6.1 `STABLE` 계약 위반

`STABLE`은 "데이터베이스를 수정하지 않는다"를 뜻한다. 그런데:

- `og_cypher_sql`(`cypher/mod.rs:74`, `stable`) → `compile_cached` → `views::ensure_view` →
  `CREATE OR REPLACE VIEW` (`views.rs:135`)
- `og_typeql_sql`(`typeql/mod.rs:82`, `stable`) → `Compiler::new` →
  `schema::ensure_has_type` → `INSERT INTO og_catalog.type` + `CREATE TABLE`
  (`typeql/schema.rs:526-552`)

읽기 전용 트랜잭션이나 스탠바이에서 이 함수들을 부르면 실패한다.
`og_apply_role`로 `default_transaction_read_only = on`을 걸면 같은 일이 벌어진다
(`agent/mod.rs:434`). → `CODE-02`.

---

## 7. 사실 — Bolt 게이트웨이의 동시성

- **커넥션 1개 = 스레드 1개 = PostgreSQL 백엔드 1개** (`bolt/src/main.rs:69-79`).
  스레드 간 공유 상태는 `Arc<Config>`(불변)뿐이다.
- 커넥션 풀이 없으므로 Bolt 커넥션 수가 곧 PostgreSQL 커넥션 수다.
  `max_connections`가 사실상의 한계다.
- 백엔드가 세션에 고정되므로 `og_cypher_stats()`와 CSR 같은 백엔드-로컬 상태가
  Bolt 세션 수명 동안 일관된다. 이건 의도된 것이다 (`session.rs:368-372`).
- `RESET`은 열린 트랜잭션을 롤백한다 (`session.rs:198-203`).

---

## 8. 결정 요약

| 결정 | 근거 | 대가 |
|---|---|---|
| 트랜잭션을 시작/종료하지 않는다 | 확장은 호출자의 트랜잭션 안에 산다 | 부분 실패를 세밀하게 다룰 수 없음 |
| 명시적 락 없음 | PostgreSQL 기본값에 의존 | MERGE/put이 원자적이 아님 (4.3, 4.4) |
| 식별자 발급이 사실상의 직렬화 지점 | `storage/mod.rs:24-34` | 타입 단위 쓰기 병목 |
| 백엔드-로컬 캐시 3종 | 성능 | 롤백/스키마 변경과 불일치 (`CODE-01`) |
| CSR은 스냅샷을 얼린다 | `traverse.rs:19-23` | RLS 우회, 최신성 없음 — **문서화된 트레이드오프** |
| 쓰기 경로가 DDL을 유발할 수 있다 | Neo4j처럼 스키마 없이 쓸 수 있어야 함 | 초기 적재 시 락 경합 |

---

## 금지 / 필수

- **금지**: 확장 코드 안에서 `BEGIN` / `COMMIT` / `ROLLBACK` / `SAVEPOINT`를 실행하는 것.
  호출자의 트랜잭션을 깨뜨린다.
- **금지**: `SpiClient`나 `SpiTupleTable`을 `Spi::connect` 클로저 밖으로 반출하는 것.
  결과는 `Vec`으로 물질화한 뒤 반환한다.
- **금지**: `#[pg_extern(stable)]`이나 `immutable` 함수에서 DDL/DML을 하는 것.
  현재 위반 2건이 있다 (6.1절).
- **금지**: 백엔드-로컬 캐시(`PLAN_CACHE`, `CSR`)에 **트랜잭션 정합성이 필요한** 정보를 넣는 것.
- **필수**: `MERGE` / `put`의 멱등성이 필요하면 **고유 인덱스를 반드시 선언한다.**
  코드는 이를 보장하지 않는다.
- **필수**: 대량 적재 전에 스키마를 먼저 선언한다 (`og_create_type` / `og_add_property` /
  TypeQL `define`). 쓰기 경로에서 DDL이 일어나지 않게 하는 유일한 방법이다.
- **필수**: `og_csr_build()`를 쓴 뒤 데이터를 바꿨으면 다시 빌드한다. RLS가 걸린
  그래프에서는 `og_csr_*`를 쓰지 않는다.

<!-- affects: backend, operations, data -->
<!-- requires-update: 08_operations/, 06_data/ -->
