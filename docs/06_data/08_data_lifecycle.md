# 08. 데이터 생애주기 — 생성 → 사용 → 폐기

> **이 문서가 답하는 질문**
> - 노드/엣지 하나가 만들어질 때 몇 개의 관계가 동시에 바뀌는가?
> - 그래프를 드롭하면 무엇이 사라지고 **무엇이 남는가**?
> - 타입을 드롭할 때 캐스케이드는 어디까지 가는가?
> - 재구성(`og_reorganize`) / 무결성 검사(`og_check_integrity`)는 무엇을 보는가?
> - 이력·시점 조회는 어떤 대가를 요구하는가?
> - VACUUM / TOAST / `pg_dump`와 이 스키마는 어떻게 상호작용하는가?

**정본**: [`engine/src/storage/mod.rs`](../../engine/src/storage/mod.rs),
[`engine/src/catalog/types.rs:321-340, 684-711`](../../engine/src/catalog/types.rs),
[`engine/src/storage/stats.rs:117-263`](../../engine/src/storage/stats.rs),
[`engine/src/agent/mod.rs:443-526`](../../engine/src/agent/mod.rs).

---

## 1. 생성

### 노드 하나가 태어날 때

```rust
pub fn create_node_inner(_gid: i32, tid: i32, type_name: &str, props: Value) -> i64 {
    // ① 추상 타입 거부
    // ② alloc_id(tid)                          — og_id_alloc UPSERT (행 락)
    // ③ plan_props(tid, &props, "$2")          — og_catalog.property 조회
    //                                            (+ 필요하면 og_add_property DDL)
    // ④ INSERT INTO og_data.og_node (id, type_id)
    // ⑤ INSERT INTO {table} (id, <컬럼들>, __ext)
}
```
(`engine/src/storage/mod.rs:253-291`)

**최소 SPI 문장 수: 4** (타입 조회 2 + og_node 1 + 타입 테이블 1) — 실제로는
`type_kind`, `storage_table`, `plan_props`의 property 조회까지 포함해 더 많다.
새 프로퍼티가 있으면 `og_add_property()` DDL이 추가로 돈다.

### 엣지 하나가 태어날 때

```rust
pub fn create_edge_inner(...) -> i64 {
    validate_roles(tid, rel_type, src, dst);   // 역할 조회 + og_is_subtype (SPI)
    let eid = alloc_id(tid);                    // og_id_alloc UPSERT
    // INSERT INTO og_data.og_edge (id, type_id, src, dst)
    // INSERT INTO {table} (id, src, dst, <컬럼들>, __ext)
    adjacency::append(src, tid, 'o', dst, eid); // UPDATE 또는 INSERT (+ max(seq) 서브쿼리)
    adjacency::append(dst, tid, 'i', src, eid); // 같은 것 한 번 더
}
```
(`engine/src/storage/mod.rs:402-452`)

**엣지 하나 = 최소 6개의 SPI 문장**, 그중 2개는 `og_adj` 튜플 전체 재작성이다
(→ [`03_adjacency_model.md`](03_adjacency_model.md)의 쓰기 증폭).

주석이 트랜잭션 원자성을 명시한다:
"Both adjacency directions, same transaction — spec 001 FR-012"
(`engine/src/storage/mod.rs:444`).

### 벌크 로드

**공개 벌크 로드 API가 없다.** `#[pg_extern]` 함수 목록에 `og_bulk_*` 계열이 없고,
`COPY`를 쓰는 경로도 없다(확인: `engine/src/`에 `COPY` 매치 0건).

RDF 로더(`og_load_rdf`)도 트리플마다 `create_node_inner` / `create_edge_inner`를
탄다. 벤치마크 하네스는 **이 경로를 완전히 우회한다**:

> "Bulk load through SQL rather than one Cypher CREATE per row: the
> per-statement overhead would dominate and tell us nothing."
> (`bench/harness.py:322-323`)

하네스가 쓰는 것은 `INSERT ... SELECT` + `array_agg` 재청킹이다
(`bench/harness.py:327-355`). `docs/benchmark.md:325`가 이 경로로 124,580 edges/s를
보고한다. **`og_create_edge()` 경로의 처리량은 이 저장소 어디에도 측정치가 없다.**
→ `PERF-15`

**운영에서 벌크 로드를 하려면** 하네스와 같은 형태를 직접 써야 하며,
그때 지켜야 할 것:

```sql
-- 0) 전제: 타입과 프로퍼티는 미리 선언되어 있어야 한다.
--    (실컬럼이 없으면 값이 __ext로 떨어진다)

-- 1) 노드 레지스트리 + 타입 테이블
INSERT INTO og_data.og_node (id, type_id)
SELECT og_make_id(0, :tid, i), :tid FROM generate_series(1, :n) i;
INSERT INTO og_data.n_:tid (id, p_name) SELECT og_make_id(0, :tid, i), 'n'||i
  FROM generate_series(1, :n) i;

-- 2) 엣지 레지스트리 + 타입 테이블
--    (og_make_id로 id를 직접 만든다)

-- 3) 인접 세그먼트 — 양방향 모두. 한 번에 완성된 형태로.
INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
SELECT src, :rid, 'o', chunk, count(*)::int4, array_agg(dst), array_agg(id)
  FROM (SELECT src, dst, id,
               ((row_number() OVER (PARTITION BY src ORDER BY id)) - 1)::int4 / 256 AS chunk
          FROM og_data.og_edge WHERE type_id = :rid) x
 GROUP BY src, chunk;
INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
SELECT dst, :rid, 'i', chunk, count(*)::int4, array_agg(src), array_agg(id)
  FROM (SELECT src, dst, id,
               ((row_number() OVER (PARTITION BY dst ORDER BY id)) - 1)::int4 / 256 AS chunk
          FROM og_data.og_edge WHERE type_id = :rid) x
 GROUP BY dst, chunk;

-- 4) ★ 하네스에는 없지만 운영에서는 필수 — id 워터마크 복구
INSERT INTO og_data.og_id_alloc (type_id, next_id)
SELECT type_id, max(og_id_local(id)) + 1 FROM og_data.og_node  GROUP BY type_id
UNION ALL
SELECT type_id, max(og_id_local(id)) + 1 FROM og_data.og_edge  GROUP BY type_id
ON CONFLICT (type_id) DO UPDATE
   SET next_id = GREATEST(og_id_alloc.next_id, EXCLUDED.next_id);

-- 5) ★ 통계
ANALYZE og_data.og_adj;
ANALYZE og_data.og_node;
ANALYZE og_data.og_edge;
-- 타입 테이블도 각각

-- 6) 검증
SELECT * FROM og_check_integrity();
```

**(4)를 빠뜨리면 이후 `og_create_node()`가 이미 존재하는 id를 발급한다.**
벤치 하네스는 읽기 전용이라 이 단계가 없다.

---

## 2. 사용 중 — 갱신

### 프로퍼티 갱신

```sql
UPDATE {table} SET <컬럼>=..., __ext = COALESCE(__ext,'{}') || COALESCE({ext},'{}')
 WHERE id = $1
```
(`engine/src/storage/mod.rs:320-323`)

- 선언된 컬럼은 덮어쓴다.
- `__ext`는 **병합**한다 — 키 삭제 경로가 없다.
- `plan_props`가 매번 호출되므로 **모든 갱신이 컬럼 승격을 촉발할 수 있다**.
  타입이 어긋나면 `ALTER TABLE ... TYPE text` 전체 재작성이 이 자리에서 시작된다
  (→ [`05_property_model.md`](05_property_model.md)).

### 인접 갱신

엣지를 새로 만들면 `og_adj` 튜플 두 개가 재작성된다.
**이것이 `og_adj`의 죽은 튜플 발생원 1위다.** fillfactor 80은 꽉 찬 세그먼트에는
도움이 되지 않으므로(→ [`03`](03_adjacency_model.md)) 새 페이지로 이동한다.

---

## 3. 폐기 — 노드 삭제

```rust
pub fn delete_node_inner(id: i64) -> i64 {
    // ① SELECT DISTINCT e FROM og_adj a, LATERAL unnest(a.eid) e WHERE a.src = $1
    //    → 양방향 모두 걸리므로 인접 엣지 전부
    for eid in incident { removed += delete_edge_inner(eid); }
    // ② DELETE FROM {table} WHERE id = $1
    // ③ DELETE FROM og_data.og_node WHERE id = $1
    // ④ DELETE FROM og_data.og_adj  WHERE src = $1
}
```
(`engine/src/storage/mod.rs:355-383`)

`delete_edge_inner` 하나가 다시 6개 문장이다(`engine/src/storage/mod.rs:501-528`):
`og_edge` 조회 → `adjacency::remove` ×2 (각각 UPDATE + DELETE = 4문장) →
타입 테이블 DELETE → `og_edge` DELETE → `og_role_player` DELETE.

**따라서 차수 D인 노드를 지우는 비용은 약 `6D + 4` 개의 SPI 문장**이고,
그중 2D 개가 `og_adj` 튜플 전체 재작성이다.

차수 10,000인 슈퍼노드 하나를 지우면 6만 개 이상의 문장이 한 트랜잭션 안에서 돈다.
→ `PERF-16`

**주의**: ④가 `WHERE src = $1`만 지운다. 이 노드가 **다른 노드의 이웃 배열에**
남아 있을 가능성은 ①→`delete_edge_inner`가 막아준다 — 인접 엣지를 모두 지우면서
반대편 세그먼트도 정리하기 때문이다. 하지만 인접 세그먼트가 이미 어긋나 있으면
(벌크 로드 실수 등) 고아 참조가 남는다. `og_check_integrity()`의 검사 1이 이를 잡는다.

**정리되지 않는 것**:
| 테이블 | 정리 여부 |
|---|---|
| `og_data.og_role_player` (`player_id`로 참여 중인 행) | **안 됨** — `edge_id` 기준으로만 지운다 |
| `og_data.og_embedding_state` | **안 됨** |
| `og_data.og_source` | **안 됨** |
| `og_data.og_iri` | **안 됨** |
| `og_data.og_history` | 안 됨 (의도적 — 이력이다) |

→ `DATA-07`

---

## 4. 폐기 — 타입 드롭

```rust
fn og_drop_type(graph: &str, name: &str, cascade: bool) {
    let subs = labeling::og_subtypes(tid);
    // 인스턴스 수 확인 (서브타입 포함). cascade가 아니면 거부
    for sub in subs {
        DROP TABLE IF EXISTS {storage_table} CASCADE
        DELETE FROM og_data.og_node  WHERE type_id = $1
        DELETE FROM og_data.og_edge  WHERE type_id = $1
        DELETE FROM og_data.og_adj   WHERE etype  = $1      -- ★ 순차 스캔
        DELETE FROM og_catalog.type  WHERE type_id = $1     -- ★ 캐스케이드 시작점
    }
    labeling::relabel_graph(gid);
}
```
(`engine/src/catalog/types.rs:685-711`)

### 캐스케이드가 가는 곳

`DELETE FROM og_catalog.type`가 트리거하는 것:

| 자식 테이블 | FK 컬럼 | 인덱스 있는가 |
|---|---|---|
| `type_parent` | `type_id` | 예 (PK 선두) |
| `type_parent` | `parent_id` | 예 (`type_parent_parent_idx`) |
| `type_label` | `type_id` | 예 (PK 선두) |
| `property` | `type_id` | 예 (`UNIQUE (type_id, name)` 선두) |
| `role` | `rel_type_id` | 예 (`UNIQUE (rel_type_id, name)` 선두) |
| `role` | `player_type_id` | **아니오** → 자식 테이블 전체 스캔 |
| `og_constraint` | `type_id` | **아니오** → 전체 스캔 |
| `rule` | `rel_type_id` | 예 (UNIQUE 선두) |
| `rule` | `target_type_id` | **아니오** → 전체 스캔 |
| `embedding` | `type_id` | 예 (`UNIQUE (type_id, prop)` 선두) |
| `mapping` | `type_id` | 예 (PK) |

**인덱스가 없는 FK는 부모 행 삭제마다 자식 테이블을 순차 스캔한다**
(그리고 그 행들에 대한 잠금을 잡는다). 카탈로그 테이블은 보통 작지만,
`role.player_type_id`는 `NO ACTION`이므로 **참조가 남아 있으면 삭제 자체가 실패**한다.
`role.player_type_id`를 가리키는 타입을 지우면 `og_drop_type`이 FK 위반으로 죽는다.
→ `DATA-05`

### 정리되지 않는 것

| 대상 | 상태 |
|---|---|
| `og_data.og_role_player` | **남는다** — 삭제된 역할을 가리키는 고아 행 |
| `og_data.og_embedding_state` | **남는다** |
| `og_data.og_source` | **남는다** |
| `og_data.og_iri` | **남는다** |
| `og_data.og_history` | 남는다 (의도적) |
| `og_data.og_id_alloc` | **남는다** — `type_id`가 재사용되지 않으므로 무해하지만 누적 |
| `og_data.og_adj` 중 이 타입 노드가 **다른 타입 엣지의 src인** 세그먼트 | **남는다** — `etype`으로만 지우므로 |

마지막 항목이 가장 위험하다. 타입 `A`를 지웠는데 `A` 인스턴스가 타입 `B`의
엣지 끝점이었다면, `og_adj`의 `(src=<지워진 A 노드>, etype=B, ...)` 세그먼트가
그대로 남는다. `og_check_integrity()`의 검사 1(`dangling_adjacency`)이 이를 잡는다.

---

## 5. 폐기 — 그래프 드롭 ★

```rust
fn og_drop_graph(name: &str) {
    let gid = graph_id(name);
    // 이 그래프의 모든 storage_table을 DROP TABLE ... CASCADE
    DELETE FROM og_catalog.graph WHERE graph_id = $1
}
```
(`engine/src/catalog/types.rs:321-340`)

**`og_data`의 어떤 행도 지우지 않는다.**

`DELETE FROM og_catalog.graph`가 FK 캐스케이드로 `og_catalog.type`을 지우고,
거기서 다시 `type_parent` / `type_label` / `property` / `role` / `og_constraint` /
`rule` / `embedding` / `mapping`을 지운다. 하지만 `og_data`에는 FK가 없다.

**결과 상태**
| 테이블 | 남는가 |
|---|---|
| `og_data.og_node` | **전부 남음** — 이제 존재하지 않는 `type_id`를 가리킨다 |
| `og_data.og_edge` | **전부 남음** |
| `og_data.og_adj` | **전부 남음** |
| `og_data.og_id_alloc` | **전부 남음** |
| `og_data.og_role_player` | **전부 남음** |
| `og_data.og_embedding_state` / `og_source` / `og_iri` / `og_history` / `og_audit` / `og_triple_overflow` | **전부 남음** |

`og_check_integrity()`의 검사 4가 정확히 이 상태를 `orphan_node`로 보고한다
(`engine/src/storage/stats.rs:244-259`).

**증상**: 그래프를 지웠는데 `og_data.og_adj`가 그대로 디스크를 먹고, `og_reorganize`는
`og_node → type` 조인을 통과하지 못해 이 세그먼트들을 정리하지 못한다
(`engine/src/storage/stats.rs:128-130`).

**더 나쁜 것**: `og_catalog.type_id_seq`는 되감기지 않으므로 같은 이름의 그래프를
다시 만들면 **새 `type_id`**를 받는다. 따라서 남은 고아 데이터가 새 그래프와
충돌하지는 않는다 — 조용히 낭비될 뿐이다.

→ `DATA-06`. **임시 대응 SQL**은 이 문서 마지막 절에 있다.

---

## 6. 재구성 — `og_reorganize(graph)`

→ 상세는 [`03_adjacency_model.md`](03_adjacency_model.md).

| 항목 | 값 | 근거 |
|---|---|---|
| 대상 | 세그먼트 2개 이상이고 하나를 줄일 수 있는 `(src, etype, dir)` | `engine/src/storage/stats.rs:132` |
| 범위 | 해당 그래프에 속한 노드만 | `engine/src/storage/stats.rs:128-130` |
| 단위 | `(src, etype, dir)` 하나당 SQL 한 문장 | `engine/src/storage/stats.rs:145-162` |
| 트랜잭션 | **전체가 한 트랜잭션** | Rust 루프 안의 `Spi::run_with_args` |
| 반환 | 재포장한 그룹 수 | `engine/src/storage/stats.rs:163-165` |
| 읽기 차단 | 없음 (MVCC) | 주석 `engine/src/storage/stats.rs:119-120` |
| 공간 회수 | **없음** — VACUUM이 필요하다 | 삭제된 튜플은 죽은 채로 남는다 |

**언제 도는지 판단**:
```sql
SELECT og_graph_stats('mygraph') -> 'adjacency';
-- {"segments": N, "avg_fill": F, "chunk_size": 256,
--  "packing_ratio": F/256, "chunked_supernodes": M}
```
(`engine/src/storage/stats.rs:69-82`)
`packing_ratio`가 1.0에서 크게 떨어졌으면 재구성이 도움이 된다.

**반드시 뒤이어**: `VACUUM (ANALYZE) og_data.og_adj;`

---

## 7. 무결성 검사 — `og_check_integrity()`

4개 검사, 각각 `LIMIT 100`, 이상 없으면 0행이 정답이다
(`engine/src/storage/stats.rs:168-263`).

| # | 이름 | 무엇을 보는가 | 비용 |
|---|---|---|---|
| 1 | `dangling_adjacency` | `og_adj.eid`가 없는 엣지를 가리킴 | `og_adj` 전체 스캔 × unnest, 엣지당 PK 프로브 |
| 2 | `missing_adjacency` | 엣지가 한쪽 끝에서 도달 불가 | `og_edge` 전체 스캔 × `og_adj` 프로브 2회 |
| 3 | `segment_length_mismatch` | `n ≠ array_length(nbr)` 또는 두 배열 길이 불일치 | `og_adj` 전체 스캔 |
| 4 | `orphan_node` | `og_node.type_id`가 카탈로그에 없음 | `og_node` 전체 스캔 |

**검사하지 않는 것** (중요):
- `og_edge`의 `src`/`dst`가 실제 노드인지
- `og_role_player`의 `edge_id` / `player_id` / `role_id`가 유효한지
- `og_adj.nbr` 원소가 실제 노드인지
- 양방향 세그먼트의 `nbr`이 서로 대칭인지
- `og_embedding_state` / `og_source` / `og_iri`의 고아
- `og_catalog.property.column_name`이 실제 컬럼과 일치하는지

**비용 주의**: `LIMIT 100`은 **출력**을 제한하지 검사를 제한하지 않는다.
그래프가 건강하면 100행을 못 채우므로 **전부 스캔한다.** 검사 2는
`og_edge` 전체에 대해 `og_adj` 인덱스 프로브를 두 번씩 하므로 O(|E|)다.
100만 엣지 그래프에서는 유지보수 창에서만 돌릴 것. → `PERF-17`

---

## 8. 이력과 시점 조회

### 켜기

```rust
fn og_enable_history(graph: &str, type_name: &str) {
    for sub in og_subtypes(tid) {
        CREATE OR REPLACE TRIGGER og_hist_{sub}
          AFTER INSERT OR UPDATE OR DELETE ON {table}
          FOR EACH ROW EXECUTE FUNCTION og_capture_history()
    }
    // og_catalog.setting 에 'history.<graph>.<type>' = 'on' 기록
}
```
(`engine/src/agent/mod.rs:448-468`)

**기본은 꺼져 있다.** "Off by default: it costs writes"(`engine/src/agent/mod.rs:447`).

**나중에 만든 서브타입에는 트리거가 안 붙는다.** `og_enable_history`는
호출 시점의 `og_subtypes(tid)`만 순회하고, `og_create_type`은 이력 설정을 보지 않는다.

### 트리거가 하는 일

```sql
UPDATE og_data.og_history SET valid_to = now()
 WHERE entity_id = eid AND valid_to IS NULL;

INSERT INTO og_data.og_history (entity_id, is_edge, op, payload)
VALUES (eid, doc ? 'src', op, doc);
```
(`engine/sql/access.sql:288-292`)

**행 하나가 바뀔 때마다 두 개의 문장 + `og_history` 행 하나.**

- `UPDATE`는 `og_history_entity_idx (entity_id, recorded_at DESC)`로 진입해
  `valid_to IS NULL` 필터를 건다. 그 엔티티의 이력이 길수록 스캔이 길어진다.
- `payload = to_jsonb(NEW)`는 **행 전체**다. 임베딩 컬럼이 있으면
  벡터 전체가 매번 jsonb로 직렬화된다. → `PERF-12`
- `is_edge`를 `doc ? 'src'`로 판정한다 — `src`라는 이름의 프로퍼티를 가진
  **노드**는 엣지로 오분류된다. 프로퍼티 컬럼은 `p_src`가 되므로
  실제로는 `e_<tid>` 테이블의 `src` 컬럼만 해당한다. 안전하다.

### 시점 조회

```rust
fn og_as_of(id: i64, at: TimestampWithTimeZone) -> JsonB {
    // 이력이 하나도 없으면 error!
    //   "no history is retained for entity {id}. enable it with og_enable_history(graph, type)
    //    — returning the current value instead would be a lie"
    SELECT payload FROM og_data.og_history
     WHERE entity_id = $1 AND recorded_at <= $2
     ORDER BY recorded_at DESC LIMIT 1
}
```
(`engine/src/agent/mod.rs:502-526`)

**좋은 설계**: 이력이 없으면 현재값을 반환하지 않고 **거부한다**
(`engine/src/agent/mod.rs:511-516`). 조용한 거짓말보다 시끄러운 오류가 낫다.

**한계**
- 인덱스는 `(entity_id, recorded_at DESC)`이므로 이 질의는 최적이다.
- 반환값은 **물리 컬럼 이름 그대로의 jsonb**다 (`p_title`, `__ext`, …).
  `og_node_json()`처럼 프로퍼티 이름으로 바꿔주지 않는다.
- 그래프 전체의 시점 스냅샷은 없다. 엔티티 단위만이다.
- `og_history`에는 **보존 정책이 없다.** 무한히 자란다.

---

## 9. VACUUM / TOAST / `pg_dump`

### VACUUM

**가장 필요한 테이블 순서**:

| 테이블 | 왜 |
|---|---|
| `og_data.og_adj` | 엣지 하나 추가/삭제마다 튜플 전체 재작성 → 죽은 튜플 대량 발생 |
| `og_data.og_history` (켠 경우) | 갱신마다 `UPDATE` + `INSERT` |
| 타입 테이블 `n_*` / `e_*` | 프로퍼티 갱신 |

기본 autovacuum 임계값(`autovacuum_vacuum_scale_factor = 0.2`)은
`og_adj`의 접근 패턴에 비해 느슨하다. 권장:

```sql
ALTER TABLE og_data.og_adj SET (
    autovacuum_vacuum_scale_factor  = 0.02,
    autovacuum_analyze_scale_factor = 0.02,
    autovacuum_vacuum_cost_delay    = 0
);
```

> **미측정**: 위 값은 접근 패턴 분석에서 나온 제안이며 이 저장소에서 실측하지 않았다.
> 실제 죽은 튜플 비율을 먼저 볼 것:
> ```sql
> SELECT relname, n_live_tup, n_dead_tup,
>        round(n_dead_tup::numeric / GREATEST(n_live_tup,1) * 100, 1) AS dead_pct,
>        last_vacuum, last_autovacuum, last_analyze, last_autoanalyze
>   FROM pg_stat_user_tables WHERE schemaname = 'og_data'
>  ORDER BY n_dead_tup DESC LIMIT 20;
> ```

### TOAST

| 컬럼 | 저장 전략 | TOAST로 나가는가 |
|---|---|---|
| `og_adj.nbr` / `eid` | `MAIN` (명시) | 정상 경로에서는 **아니오** (→ [`03`](03_adjacency_model.md)) |
| `n_*.p_embedding` (`vector(N)`) | `EXTENDED` (기본) | **예** — `vector(1536)`은 6KB |
| `n_*.__ext` (jsonb) | `EXTENDED` (기본) | 큰 페이로드면 예 |
| `og_history.payload` (jsonb) | `EXTENDED` (기본) | 대개 예 |
| `og_audit.query` (text) | `EXTENDED` (기본) | 긴 질의면 예 |

### `pg_dump`

등록 검증 결과는 [`01_physical_schema.md`](01_physical_schema.md)에 있다 —
부트스트랩의 27개 테이블과 11개 시퀀스가 **전부** 등록되어 있다.

**주의할 점**
- 런타임 생성 테이블(`n_*`, `e_*`, `a_*`)과 뷰(`v_*`, `ve_*`, 별칭 뷰)는
  확장 소유가 아니므로 평범한 사용자 객체로 덤프된다(`engine/sql/bootstrap.sql:400-401`).
- `v_*` / `ve_*` 뷰는 **복원 후 그냥 두어도 된다.** 다음 스키마 변경이 전부 드롭하고
  `ensure_view()`가 다시 만든다.
- `og_catalog.setting`은 시드 4개 키를 제외하고 덤프된다
  (`engine/sql/bootstrap.sql:420-422`). 확장 스크립트가 그 키들을 다시 넣기 때문이다.
- 이력 트리거(`og_hist_*`)는 사용자 테이블 위의 트리거이므로 덤프된다.
  트리거 함수 `og_capture_history()`는 확장 소유라 `CREATE EXTENSION`이 만든다.

> **미확인**: 실제 `pg_dump` → `pg_restore` 왕복은 이 문서 작성 시점에 실행하지 않았다.

---

## 운영 레시피

### A. 고아 데이터 정리 (`og_drop_graph` 이후)

> **먼저 백업할 것.** 아래는 카탈로그에 없는 `type_id`를 가진 모든 `og_data` 행을 지운다.
> 살아 있는 그래프가 있는 DB에서 그대로 돌리기 전에 `SELECT count(*)`로 먼저 확인할 것.

```sql
BEGIN;

-- 확인
SELECT 'og_node' AS t, count(*) FROM og_data.og_node n
  WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = n.type_id)
UNION ALL
SELECT 'og_edge', count(*) FROM og_data.og_edge e
  WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = e.type_id)
UNION ALL
SELECT 'og_adj', count(*) FROM og_data.og_adj a
  WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = a.etype);

-- 정리
DELETE FROM og_data.og_adj a
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = a.etype);
DELETE FROM og_data.og_edge e
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = e.type_id);
DELETE FROM og_data.og_node n
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = n.type_id);
DELETE FROM og_data.og_id_alloc i
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type c WHERE c.type_id = i.type_id);
DELETE FROM og_data.og_role_player rp
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.role r WHERE r.role_id = rp.role_id);
DELETE FROM og_data.og_embedding_state s
 WHERE NOT EXISTS (SELECT 1 FROM og_data.og_node n WHERE n.id = s.entity_id);
DELETE FROM og_data.og_source o
 WHERE NOT EXISTS (SELECT 1 FROM og_data.og_node n WHERE n.id = o.entity_id);
DELETE FROM og_data.og_iri x
 WHERE NOT EXISTS (SELECT 1 FROM og_data.og_node n WHERE n.id = x.entity_id);

COMMIT;
VACUUM (ANALYZE) og_data.og_adj;
```

**주의**: `og_data.og_adj`에 `etype` 인덱스가 없으므로 위 DELETE는 순차 스캔이다.

### B. 정기 점검 (주 1회)

```sql
-- 1. 무결성
SELECT * FROM og_check_integrity();

-- 2. 조각화
SELECT og_graph_stats('mygraph') -> 'adjacency';

-- 3. 슈퍼노드
SELECT og_degree_distribution('mygraph');

-- 4. id 압박
SELECT last_value AS types_used, 262143 - last_value AS types_left
  FROM og_catalog.type_id_seq;
SELECT a.type_id, t.name, a.next_id,
       round(a.next_id::numeric / 68719476736 * 100, 4) AS local_pct
  FROM og_data.og_id_alloc a LEFT JOIN og_catalog.type t USING (type_id)
 ORDER BY a.next_id DESC LIMIT 10;

-- 5. 죽은 튜플
SELECT relname, n_live_tup, n_dead_tup
  FROM pg_stat_user_tables WHERE schemaname = 'og_data'
 ORDER BY n_dead_tup DESC LIMIT 10;

-- 6. 감사/이력 크기
SELECT pg_size_pretty(pg_total_relation_size('og_data.og_audit'))   AS audit,
       pg_size_pretty(pg_total_relation_size('og_data.og_history')) AS history;
```

---

## 금지 / 필수

**금지**
- **`og_drop_graph()`를 호출한 뒤 정리 없이 두는 것.** `og_data` 전체가 남는다.
- 슈퍼노드를 `og_delete_node()`로 지우는 것. 차수당 6문장이 한 트랜잭션에 쌓인다.
  먼저 엣지를 배치로 정리할 것.
- `og_reorganize()` 뒤에 `VACUUM`을 생략하는 것. 공간이 회수되지 않는다.
- 프로덕션 트래픽 중에 `og_check_integrity()`를 도는 것. O(|E|)다.
- 벌크 로드 후 `og_id_alloc` 갱신을 빠뜨리는 것. **id가 충돌한다.**
- `og_history` / `og_audit`에 보존 정책 없이 이력을 켜두는 것.

**필수**
- 벌크 로드: 타입/프로퍼티 선언 → 데이터 → `og_adj` 양방향 → `og_id_alloc` 갱신 →
  `ANALYZE` → `og_check_integrity()`. 순서 전부.
- 임베딩 대량 적재 시 HNSW 인덱스 드롭 → 적재 → 재생성.
- `og_adj`의 autovacuum을 공격적으로 조정할 것.
- 정기 점검을 자동화할 것 (위 B절).

---

<!-- affects: data, ops, backend -->
<!-- requires-update: docs/06_data/10_improvements_data.md -->
